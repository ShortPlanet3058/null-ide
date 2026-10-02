//! The Cmd/Ctrl+I card: describe a change to some code, see it as a diff, accept or reject.

use crate::ai::{self, AiEvent, Prompt};
use crate::fonts::Fonts;
use crate::settings::Settings;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use futures::StreamExt;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, ScrollHandle, Subscription,
    Task, Window, actions, div, prelude::*, px,
};
use std::path::PathBuf;

actions!(inline_assist, [Submit, Accept, Dismiss]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("InlineAssist");
    cx.bind_keys([
        KeyBinding::new("enter", Submit, ctx),
        KeyBinding::new("tab", Accept, ctx),
        KeyBinding::new("secondary-enter", Accept, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
    ]);
}

/// Whole files above this many lines are trimmed to the lines around the target.
const CONTEXT_LINES: usize = 600;

enum State {
    Asking,
    Writing,
    Ready,
    Failed(String),
}

pub enum InlineAssistEvent {
    /// The person accepted: replace the target with this text.
    Accept(String),
    Dismiss,
}

/// What the card works on: the code to change and the file around it.
pub struct AssistTarget {
    pub path: Option<PathBuf>,
    pub language: &'static str,
    /// Zero-based lines, end exclusive.
    pub lines: std::ops::Range<usize>,
    pub original: String,
    /// The file, trimmed to the lines around the target when it's long.
    pub context: String,
}

pub struct InlineAssist {
    input: Entity<TextInput>,
    target: AssistTarget,
    proposal: String,
    state: State,
    task: Option<Task<()>>,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<InlineAssistEvent> for InlineAssist {}

impl InlineAssist {
    pub fn new(target: AssistTarget, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Describe the change", cx));
        let subscription = cx.subscribe(&input, |this, _, TextInputEvent::Changed, cx| {
            // Editing the request after an answer goes back to asking.
            if matches!(this.state, State::Failed(_)) {
                this.state = State::Asking;
                cx.notify();
            }
        });
        let state = match cx.global::<Settings>().ai.active() {
            ai::ProviderId::Off => State::Failed("Choose where AI answers come from in Settings → AI (⌘,).".into()),
            _ => State::Asking,
        };
        Self {
            input,
            target,
            proposal: String::new(),
            state,
            task: None,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    }

    pub fn lines(&self) -> std::ops::Range<usize> {
        self.target.lines.clone()
    }

    pub fn trim_context(text: &str, lines: &std::ops::Range<usize>) -> String {
        let all: Vec<&str> = text.lines().collect();
        if all.len() <= CONTEXT_LINES {
            return text.to_string();
        }
        let margin = CONTEXT_LINES / 2;
        let start = lines.start.saturating_sub(margin);
        let end = (lines.end + margin).min(all.len());
        format!("[… lines 1-{start} omitted …]\n{}\n[… rest of the file omitted …]", all[start..end].join("\n"))
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        let instruction = self.input.read(cx).text().trim().to_string();
        if instruction.is_empty() || matches!(self.state, State::Writing) {
            return;
        }
        let settings = cx.global::<Settings>().ai.clone();
        let t = &self.target;
        let path = t.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "untitled".into());
        let prompt = Prompt {
            system: "You are a careful senior engineer editing part of a file in a code editor. You get the whole \
                     file for context and the part to change between <selection> tags. Apply the instruction to that \
                     part only. Reply with the complete new text for that part and nothing else: no explanation, no \
                     markdown fences, nothing before or after the code. Keep the file's indentation and style."
                .into(),
            user: format!(
                "File: {path} ({})\n\n<file>\n{}\n</file>\n\n<selection lines=\"{}-{}\">\n{}</selection>\n\nInstruction: {instruction}",
                t.language,
                t.context,
                t.lines.start + 1,
                t.lines.end,
                t.original
            ),
        };
        self.proposal.clear();
        self.state = State::Writing;
        let mut events = ai::stream(settings, prompt);
        self.task = Some(cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let keep_going = this
                    .update(cx, |this, cx| {
                        match event {
                            AiEvent::Text(text) => this.proposal.push_str(&text),
                            AiEvent::Done => {
                                this.proposal = ai::strip_code_fence(&this.proposal);
                                this.state = if this.proposal.trim().is_empty() {
                                    State::Failed("The answer was empty. Try rephrasing.".into())
                                } else {
                                    State::Ready
                                };
                            }
                            AiEvent::Failed(message) => this.state = State::Failed(message),
                        }
                        cx.notify();
                        matches!(this.state, State::Writing)
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn accept(&mut self, _: &Accept, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.state, State::Ready) {
            let mut text = self.proposal.clone();
            // Keep the line break the original part ended with.
            if self.target.original.ends_with('\n') && !text.ends_with('\n') {
                text.push('\n');
            }
            cx.emit(InlineAssistEvent::Accept(text));
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InlineAssistEvent::Dismiss);
    }

    fn render_diff(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        if self.proposal.is_empty() {
            return None;
        }
        let theme = cx.global::<Theme>();
        let proposal = ai::strip_code_fence(&self.proposal);
        let diff = similar::TextDiff::from_lines(self.target.original.as_str(), proposal.as_str());
        let rows = diff.iter_all_changes().map(|change| {
            let (mark, bg, fg) = match change.tag() {
                similar::ChangeTag::Delete => ("−", theme.git_deleted.opacity(0.12), theme.muted),
                similar::ChangeTag::Insert => ("+", theme.git_added.opacity(0.12), theme.foreground),
                similar::ChangeTag::Equal => (" ", gpui::transparent_black(), theme.muted),
            };
            div().flex().bg(bg).child(div().w(px(18.)).flex_none().text_color(theme.faint).child(mark)).child(
                div().whitespace_nowrap().text_color(fg).child(change.value().trim_end_matches('\n').to_string()),
            )
        });
        Some(
            div()
                .id("assist-diff")
                .track_scroll(&self.scroll)
                .overflow_scroll()
                .max_h(px(320.))
                .rounded(px(7.))
                .bg(theme.background)
                .py(px(6.))
                .px(px(4.))
                .font_family(cx.global::<Fonts>().code.clone())
                .text_size(px(12.5))
                .line_height(px(19.))
                .children(rows),
        )
    }
}

impl Focusable for InlineAssist {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for InlineAssist {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let diff = self.render_diff(cx);
        let theme = cx.global::<Theme>();
        let provider = cx.global::<Settings>().ai.provider.label();
        let lines = &self.target.lines;
        let scope = if lines.len() == 1 {
            format!("line {}", lines.start + 1)
        } else {
            format!("lines {}–{}", lines.start + 1, lines.end)
        };
        let status: Option<(String, gpui::Hsla)> = match &self.state {
            State::Asking => None,
            State::Writing => Some((format!("Writing with {provider}…"), theme.muted)),
            State::Ready => None,
            State::Failed(message) => Some((message.clone(), theme.error)),
        };
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .h(px(26.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .text_size(px(12.))
                .when(primary, |b| b.bg(theme.caret).text_color(theme.background))
                .when(!primary, |b| b.text_color(theme.muted).hover(|s| s.bg(theme.hairline)))
                .child(label)
        };
        let ready = matches!(self.state, State::Ready);
        div()
            .key_context("InlineAssist")
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::accept))
            .on_action(cx.listener(Self::dismiss))
            .occlude()
            .w(px(640.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(10.))
            .rounded(px(11.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.hairline)
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(11.5))
                    .text_color(theme.faint)
                    .child(div().size(px(6.)).rounded_full().bg(theme.caret))
                    .child(format!("Edit {scope}"))
                    .child(div().flex_1())
                    .child(provider),
            )
            .child(
                div()
                    .h(px(30.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(7.))
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.hairline)
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .child(self.input.clone()),
            )
            .children(diff)
            .children(status.map(|(text, color)| div().text_size(px(12.)).text_color(color).child(text)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .text_color(theme.faint)
                    .child(if ready {
                        "Tab to accept · Esc to reject · edit the request and ↵ to try again"
                    } else {
                        "↵ to send · Esc to cancel"
                    })
                    .child(div().flex_1())
                    .when(ready, |row| {
                        row.child(
                            button("reject", "Reject", false)
                                .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.dismiss(&Dismiss, w, cx))),
                        )
                        .child(
                            button("accept", "Accept", true)
                                .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.accept(&Accept, w, cx))),
                        )
                    }),
            )
    }
}
