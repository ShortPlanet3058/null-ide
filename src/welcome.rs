//! The first thing shown on a first launch: three choices (look, shortcuts, AI),
//! each applied as soon as it's picked, then out of the way for good.

use crate::ai::ProviderId;
use crate::keymap::Keymap;
use crate::settings::{self, Settings};
use crate::theme::{Theme, ThemeName};
use crate::ui;
use gpui::{
    AnyElement, App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, Window,
    actions, div, prelude::*, px,
};

actions!(welcome, [FinishWelcome]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Welcome");
    cx.bind_keys([KeyBinding::new("enter", FinishWelcome, ctx), KeyBinding::new("escape", FinishWelcome, ctx)]);
}

pub enum WelcomeEvent {
    Finished,
}

/// What AI the person picked here; "not now" switches AI off entirely.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AiChoice {
    NotNow,
    Provider(ProviderId),
}

const AI_CHOICES: [(AiChoice, &str); 4] = [
    (AiChoice::NotNow, "Not now"),
    (AiChoice::Provider(ProviderId::ClaudeCode), "Claude Code"),
    (AiChoice::Provider(ProviderId::Codex), "Codex"),
    (AiChoice::Provider(ProviderId::Ollama), "Ollama"),
];

pub struct Welcome {
    focus_handle: FocusHandle,
}

impl EventEmitter<WelcomeEvent> for Welcome {}

impl Welcome {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { focus_handle: cx.focus_handle() }
    }

    fn finish(&mut self, _: &FinishWelcome, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| {
            s.welcomed = true;
            // "Not now" left as it was means no AI at all, not AI waiting for a provider.
            if s.ai.provider == ProviderId::Off {
                s.ai.enabled = false;
            }
        });
        cx.emit(WelcomeEvent::Finished);
    }

    fn label(text: &str, theme: &Theme) -> AnyElement {
        ui::section_heading(text, theme).pt(px(22.)).pb(px(10.)).into_any_element()
    }

    fn option(id: (&'static str, usize), text: &str, active: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px(px(13.))
            .py(px(7.))
            .rounded(px(ui::R_CONTROL))
            .border_1()
            .text_size(px(ui::T_MD))
            .cursor_pointer()
            .when(active, |d| d.border_color(theme.caret).bg(theme.accent_soft).text_color(theme.foreground))
            .when(!active, |d| {
                d.border_color(theme.line_strong).text_color(theme.muted).hover(|d| d.text_color(theme.foreground))
            })
            .child(text.to_string())
    }

    fn theme_card(name: ThemeName, active: bool, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        ui::theme_preview(name, active, theme)
            .id(name.label())
            .cursor_pointer()
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.theme = name)))
            .into_any_element()
    }
}

impl Focusable for Welcome {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Welcome {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<Settings>().clone();
        let theme = cx.global::<Theme>().clone();
        let ai_now = if settings.ai.enabled && settings.ai.provider != ProviderId::Off {
            AiChoice::Provider(settings.ai.provider)
        } else {
            AiChoice::NotNow
        };
        let themes: Vec<AnyElement> = [ThemeName::Oled, ThemeName::Graphite, ThemeName::Paper]
            .into_iter()
            .map(|name| Self::theme_card(name, settings.theme == name, &theme, cx))
            .collect();
        let keymaps =
            div().flex().flex_wrap().gap(px(8.)).children(Keymap::ALL.into_iter().enumerate().map(|(i, k)| {
                Self::option(("keymap", i), k.label(), settings.keymap == k, &theme)
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.keymap = k)))
            }));
        let ais = div().flex().flex_wrap().gap(px(8.)).children(AI_CHOICES.into_iter().enumerate().map(
            |(i, (choice, text))| {
                Self::option(("ai", i), text, ai_now == choice, &theme).on_click(cx.listener(
                    move |_, _: &ClickEvent, _, cx| {
                        settings::update(cx, |s| match choice {
                            AiChoice::NotNow => s.ai.enabled = false,
                            AiChoice::Provider(id) => {
                                s.ai.enabled = true;
                                s.ai.provider = id;
                            }
                        })
                    },
                ))
            },
        ));
        let ai_note = match ai_now {
            AiChoice::NotNow => "AI stays out of sight. Turn it on anytime from quick settings.",
            AiChoice::Provider(ProviderId::ClaudeCode) => "Uses your Claude plan through the claude command line tool.",
            AiChoice::Provider(ProviderId::Codex) => "Uses your ChatGPT plan through the codex command line tool.",
            AiChoice::Provider(_) => "Runs models on this computer. API keys and more providers are in Settings.",
        };
        div()
            .key_context("Welcome")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::finish))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(580.))
            .max_w_full()
            .p(px(32.))
            .flex()
            .flex_col()
            .rounded(px(ui::R_MODAL))
            .border_1()
            .border_color(theme.hairline)
            .bg(theme.raised)
            .shadow_lg()
            .child(
                div()
                    .text_size(px(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child("Welcome to Null"),
            )
            .child(
                div()
                    .pt(px(4.))
                    .text_size(px(ui::T_MD))
                    .text_color(theme.muted)
                    .child("Three choices, each one changeable later in Settings."),
            )
            .child(Self::label("Look", &theme))
            .child(div().flex().gap(px(14.)).children(themes))
            .child(Self::label("Shortcuts you already know", &theme))
            .child(keymaps)
            .child(div().pt(px(8.)).text_size(px(ui::T_SM)).text_color(theme.muted).child(settings.keymap.summary()))
            .child(Self::label("AI", &theme))
            .child(ais)
            .child(div().pt(px(8.)).text_size(px(ui::T_SM)).text_color(theme.muted).child(ai_note))
            .child(
                div().pt(px(28.)).flex().justify_end().child(
                    div()
                        .id("start")
                        .px(px(18.))
                        .py(px(8.))
                        .rounded(px(ui::R_CONTROL))
                        .bg(theme.caret)
                        .text_color(theme.on_accent)
                        .text_size(px(ui::T_MD))
                        .font_weight(FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .child("Start coding ↵")
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.finish(&FinishWelcome, window, cx)),
                        ),
                ),
            )
    }
}
