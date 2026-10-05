//! The first launch: "hello" in one language after another, then four short steps (how
//! Null looks, the typeface, the shortcuts, AI), each applied as soon as it's picked.
//! Shown again any time with "Welcome to Null…".

use crate::ai::{self, ProviderId};
use crate::keymap::Keymap;
use crate::settings::{self, Settings};
use crate::text_input::TextInput;
use crate::theme::{Syntax, Theme, ThemeName};
use crate::ui;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla, KeyBinding,
    SharedString, StyledText, Task, Window, WindowControlArea, actions, div, prelude::*, px,
};
use std::time::{Duration, Instant};

actions!(welcome, [WelcomeNext, WelcomeBack]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Welcome");
    cx.bind_keys([KeyBinding::new("enter", WelcomeNext, ctx), KeyBinding::new("escape", WelcomeBack, ctx)]);
}

pub enum WelcomeEvent {
    Finished,
}

/// Each "hello" stays this long before the next language's.
const HELLO_EVERY: Duration = Duration::from_millis(2200);
/// How long a hello takes to come in, and a step to slide in.
const FADE: Duration = Duration::from_millis(420);

/// "hello" in a few languages, as coloured pieces.
const HELLOS: &[(&str, &[(&str, Syntax)])] = &[
    (
        "Rust",
        &[
            ("println!", Syntax::Macro),
            ("(", Syntax::Punctuation),
            ("\"hello\"", Syntax::String),
            (");", Syntax::Punctuation),
        ],
    ),
    (
        "Python",
        &[
            ("print", Syntax::Function),
            ("(", Syntax::Punctuation),
            ("\"hello\"", Syntax::String),
            (")", Syntax::Punctuation),
        ],
    ),
    (
        "JavaScript",
        &[
            ("console", Syntax::Plain),
            (".", Syntax::Punctuation),
            ("log", Syntax::Function),
            ("(", Syntax::Punctuation),
            ("\"hello\"", Syntax::String),
            (")", Syntax::Punctuation),
        ],
    ),
    (
        "Go",
        &[
            ("fmt", Syntax::Plain),
            (".", Syntax::Punctuation),
            ("Println", Syntax::Function),
            ("(", Syntax::Punctuation),
            ("\"hello\"", Syntax::String),
            (")", Syntax::Punctuation),
        ],
    ),
    (
        "C",
        &[
            ("printf", Syntax::Function),
            ("(", Syntax::Punctuation),
            ("\"hello\\n\"", Syntax::String),
            (");", Syntax::Punctuation),
        ],
    ),
    (
        "Swift",
        &[
            ("print", Syntax::Function),
            ("(", Syntax::Punctuation),
            ("\"hello\"", Syntax::String),
            (")", Syntax::Punctuation),
        ],
    ),
    ("Ruby", &[("puts ", Syntax::Function), ("\"hello\"", Syntax::String)]),
];

/// How present AI is: the first question about it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Presence {
    Off,
    WhenAsked,
    WhileTyping,
}

const PRESENCE: [(Presence, &str, &str); 3] = [
    (Presence::Off, "Off", "No AI anywhere in Null. Turn it on later in Settings if you like."),
    (Presence::WhenAsked, "When I ask", "⌘I to change code, questions, and tasks. Nothing while you type."),
    (Presence::WhileTyping, "When I ask, and while I type", "Also suggests the rest of the line as you type."),
];

/// Where answers come from, in three groups: a plan you already pay for, an API key
/// (free tiers or paid), this computer.
const SUBSCRIPTIONS: [(ProviderId, &str); 2] =
    [(ProviderId::ClaudeCode, "Your Claude Pro or Max plan"), (ProviderId::Codex, "Your ChatGPT Plus or Pro plan")];
const API_PROVIDERS: [(ProviderId, &str); 7] = [
    (ProviderId::OpenaiCompatible, "GPT models"),
    (ProviderId::Claude, "Claude models (Anthropic)"),
    (ProviderId::Gemini, "Gemini models · free tier"),
    (ProviderId::Mistral, "Codestral for suggestions · free tier"),
    (ProviderId::Groq, "Very fast open models · free tier"),
    (ProviderId::OpenRouter, "Many models, one key"),
    (ProviderId::Nvidia, "Open models · free credits"),
];
const LOCAL: [(ProviderId, &str); 1] = [(ProviderId::Ollama, "Nothing leaves this computer")];

/// The setup's steps; where answers come from only when AI is on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StepKind {
    Look,
    Type,
    Keys,
    Presence,
    Provider,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Hello,
    Step(usize),
}

pub struct Welcome {
    focus_handle: FocusHandle,
    stage: Stage,
    /// Which hello is showing, and since when.
    hello: usize,
    hello_since: Instant,
    /// When the current step came in, for its slide.
    step_since: Instant,
    /// The API key typed for the provider picked, saved on the way out.
    key: Entity<TextInput>,
    _ticker: Task<()>,
}

impl EventEmitter<WelcomeEvent> for Welcome {}

impl Welcome {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // The hellos take turns while that screen shows.
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(HELLO_EVERY).await;
                let Ok(going) = this.update(cx, |this, cx| {
                    if this.stage == Stage::Hello {
                        this.hello = (this.hello + 1) % HELLOS.len();
                        this.hello_since = Instant::now();
                        cx.notify();
                    }
                    this.stage == Stage::Hello
                }) else {
                    return;
                };
                if !going {
                    return;
                }
            }
        });
        Self {
            focus_handle: cx.focus_handle(),
            stage: Stage::Hello,
            hello: 0,
            hello_since: Instant::now(),
            step_since: Instant::now(),
            key: cx.new(|cx| {
                let mut input = TextInput::new("Paste the API key (or later, in Settings)", cx);
                input.masked = true;
                input
            }),
            _ticker: ticker,
        }
    }

    fn go(&mut self, stage: Stage, cx: &mut Context<Self>) {
        self.stage = stage;
        self.step_since = Instant::now();
        cx.notify();
    }

    /// The steps as things stand: where answers come from only once AI is on.
    fn steps(cx: &App) -> Vec<StepKind> {
        let mut steps = vec![StepKind::Look, StepKind::Type, StepKind::Keys, StepKind::Presence];
        if cx.global::<Settings>().ai.enabled {
            steps.push(StepKind::Provider);
        }
        steps
    }

    fn next(&mut self, _: &WelcomeNext, _: &mut Window, cx: &mut Context<Self>) {
        let count = Self::steps(cx).len();
        match self.stage {
            Stage::Hello => self.go(Stage::Step(0), cx),
            Stage::Step(n) if n + 1 < count => self.go(Stage::Step(n + 1), cx),
            Stage::Step(_) => self.finish(cx),
        }
    }

    /// Esc: a step back; on the hello, skip the setup.
    fn back(&mut self, _: &WelcomeBack, _: &mut Window, cx: &mut Context<Self>) {
        match self.stage {
            Stage::Hello => self.finish(cx),
            Stage::Step(0) => self.go(Stage::Hello, cx),
            Stage::Step(n) => self.go(Stage::Step(n - 1), cx),
        }
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        // A key typed for the provider goes to the keychain, never to the settings file.
        let key = self.key.read(cx).text().trim().to_string();
        let provider = cx.global::<Settings>().ai.provider;
        if !key.is_empty()
            && provider.uses_api_key()
            && cx.global::<Settings>().ai.enabled
            && let Err(error) = ai::store_api_key(provider, &key)
        {
            eprintln!("null: couldn't save the key: {error}");
        }
        settings::update(cx, |s| {
            s.welcomed = true;
            // "Not now" left as it was means no AI at all, not AI waiting for a provider.
            if s.ai.provider == ProviderId::Off {
                s.ai.enabled = false;
            }
        });
        cx.emit(WelcomeEvent::Finished);
    }

    /// "null" with an amber caret: the wordmark.
    fn wordmark(theme: &Theme) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(3.))
            .text_size(px(ui::T_XL))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.foreground)
            .child("null")
            .child(div().w(px(2.)).h(px(17.)).rounded(px(1.)).bg(theme.caret))
            .into_any_element()
    }

    /// The amber button that moves on, with its key.
    fn primary(id: &'static str, label: &str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(36.))
            .px(px(18.))
            .rounded(px(ui::R_CONTROL + 2.))
            .bg(theme.caret)
            .text_color(theme.on_accent)
            .text_size(px(ui::T_LG))
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .child(label.to_string())
            .child(div().text_size(px(ui::T_SM)).opacity(0.7).child("↵"))
    }

    /// A choice as a card: a title, a line under it.
    fn card(
        id: (&'static str, usize),
        title: &str,
        line: &str,
        active: bool,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex()
            .flex_col()
            .gap(px(4.))
            .px(px(16.))
            .py(px(12.))
            .rounded(px(ui::R_POPOVER))
            .border_1()
            .cursor_pointer()
            .when(active, |d| d.border_color(theme.caret).bg(theme.accent_soft))
            .when(!active, |d| d.border_color(theme.line_strong).hover(|d| d.bg(theme.hairline)))
            .child(div().text_size(px(ui::T_LG)).text_color(theme.foreground).child(title.to_string()))
            .child(div().text_size(px(ui::T_SM)).text_color(theme.muted).child(line.to_string()))
    }

    /// A few lines of code in the chosen typeface and size, coloured by the theme.
    fn type_preview(settings: &Settings, theme: &Theme, font: SharedString) -> AnyElement {
        let lines: [&[(&str, Syntax)]; 4] = [
            &[
                ("fn ", Syntax::Keyword),
                ("greet", Syntax::Function),
                ("(name: ", Syntax::Plain),
                ("&str", Syntax::Type),
                (") {", Syntax::Plain),
            ],
            &[("    // a calm place to write code", Syntax::Comment)],
            &[
                ("    println!", Syntax::Macro),
                ("(", Syntax::Plain),
                ("\"hello, {name}\"", Syntax::String),
                (");", Syntax::Plain),
            ],
            &[("}", Syntax::Plain)],
        ];
        div()
            .w_full()
            .p(px(18.))
            .rounded(px(ui::R_POPOVER))
            .bg(theme.sunken)
            .border_1()
            .border_color(theme.hairline)
            .font_family(font)
            .text_size(px(settings.font_size))
            .line_height(px(settings.font_size * 1.6))
            .children(lines.iter().map(|pieces| {
                div().flex().children(pieces.iter().map(|(text, syntax)| {
                    div().text_color(theme.syntax(*syntax)).whitespace_nowrap().child(text.replace(' ', "\u{a0}"))
                }))
            }))
            .into_any_element()
    }

    fn render_hello(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let code_font = cx.global::<crate::fonts::Fonts>().code.clone();
        let t = (self.hello_since.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.);
        if t < 1. {
            window.request_animation_frame();
        }
        let eased = 1. - (1. - t).powi(3);
        let (language, pieces) = HELLOS[self.hello];
        // The caret blinks only once the line has settled in.
        let blink_on = (self.hello_since.elapsed().as_millis() / 550).is_multiple_of(2);
        let text: String = pieces.iter().map(|(t, _)| *t).collect();
        let mut highlights = Vec::new();
        let mut at = 0;
        for (piece, syntax) in pieces.iter() {
            highlights.push((
                at..at + piece.len(),
                gpui::HighlightStyle { color: Some(theme.syntax(*syntax)), ..Default::default() },
            ));
            at += piece.len();
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(Self::top_bar(None, &theme))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(22.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .opacity(eased)
                            .mt(px(10. * (1. - eased)))
                            .font_family(code_font)
                            .text_size(px(48.))
                            .child(StyledText::new(text).with_highlights(highlights))
                            .child(
                                div()
                                    .ml(px(4.))
                                    .w(px(4.))
                                    .h(px(50.))
                                    .rounded(px(2.))
                                    .bg(theme.caret)
                                    .when(t >= 1. && !blink_on, |d| d.opacity(0.)),
                            ),
                    )
                    .child(ui::section_heading(language, &theme).opacity(eased)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(16.))
                    .pb(px(48.))
                    .child(div().text_size(px(15.)).text_color(theme.muted).child("A calm place to write code."))
                    .child(
                        Self::primary("begin", "Begin", &theme).active(|s| s.opacity(0.7)).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.next(&WelcomeNext, window, cx)),
                        ),
                    )
                    .child(
                        div()
                            .id("skip")
                            .text_size(px(ui::T_MD))
                            .text_color(theme.faint)
                            .cursor_pointer()
                            .hover(|d| d.text_color(theme.muted))
                            .child("Skip setup")
                            .active(|s| s.opacity(0.7))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.finish(cx))),
                    ),
            )
            .into_any_element()
    }

    /// The top: room for the window buttons (and dragging the window), the wordmark, and
    /// how far along the setup is.
    fn top_bar(step: Option<(usize, usize)>, theme: &Theme) -> AnyElement {
        // On the window buttons' row, just after them.
        div()
            .h(px(42.))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .pl(px(92.))
            .pr(px(28.))
            .window_control_area(WindowControlArea::Drag)
            .child(Self::wordmark(theme))
            .children(step.map(|n| {
                div().text_size(px(ui::T_SM)).text_color(theme.faint).child(format!("{} of {}", n.0 + 1, n.1))
            }))
            .into_any_element()
    }

    fn render_step(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let settings = cx.global::<Settings>().clone();
        let theme = cx.global::<Theme>().clone();
        let steps = Self::steps(cx);
        let n = n.min(steps.len() - 1);
        let t = (self.step_since.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.);
        if t < 1. {
            window.request_animation_frame();
        }
        let eased = 1. - (1. - t).powi(3);
        let (title, sub, body): (&str, &str, AnyElement) = match steps[n] {
            StepKind::Look => (
                "Choose how Null looks",
                "Switch any time from ⌘K or Settings.",
                div()
                    .grid()
                    .grid_cols(3)
                    .gap(px(18.))
                    .children(ThemeName::ALL.into_iter().map(|name| {
                        div()
                            .id(name.label())
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .cursor_pointer()
                            .child(ui::theme_preview_scaled(name, settings.theme == name, &theme, 1.45))
                            .child(div().text_size(px(ui::T_SM)).text_color(theme.faint).child(name.note()))
                            .active(|s| s.opacity(0.7))
                            .on_click(
                                cx.listener(move |_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.theme = name)),
                            )
                    }))
                    .into_any_element(),
            ),
            StepKind::Type => (
                "Pick a typeface you can read all day",
                "Each is monospaced, calm and made for code. The size changes with ⌘+ and ⌘- too.",
                self.type_step(&settings, &theme, cx),
            ),
            StepKind::Keys => (
                "Which shortcuts do your hands know?",
                "Null's own keep the common VS Code ones. Change them any time in Settings.",
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(10.))
                    .children(Keymap::ALL.into_iter().enumerate().map(|(i, k)| {
                        Self::card(("keymap", i), k.label(), k.summary(), settings.keymap == k, &theme)
                            .active(|s| s.opacity(0.7))
                            .on_click(
                                cx.listener(move |_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.keymap = k)),
                            )
                    }))
                    .into_any_element(),
            ),
            StepKind::Presence => {
                let now = match (settings.ai.enabled, settings.ai.completions) {
                    (false, _) => Presence::Off,
                    (true, false) => Presence::WhenAsked,
                    (true, true) => Presence::WhileTyping,
                };
                (
                    "How present should AI be?",
                    "It never opens a panel on its own, and never changes code without showing you the change.",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .children(PRESENCE.into_iter().enumerate().map(|(i, (choice, title, line))| {
                            Self::card(("presence", i), title, line, now == choice, &theme)
                                .active(|s| s.opacity(0.7))
                                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    settings::update(cx, |s| {
                                        s.ai.enabled = choice != Presence::Off;
                                        s.ai.completions = choice == Presence::WhileTyping;
                                    })
                                }))
                        }))
                        .into_any_element(),
                )
            }
            StepKind::Provider => (
                "Where should answers come from?",
                "A plan you already have, an API key from any of these (free tiers included), or this computer.",
                self.provider_step(&settings, &theme, cx),
            ),
        };
        let last = n + 1 == steps.len();
        let dot = |i: usize| -> Hsla { if i == n { theme.caret } else { theme.line_strong } };
        let count = steps.len();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(Self::top_bar(Some((n, count)), &theme))
            .child(
                div().flex_1().min_h_0().flex().justify_center().items_center().px(px(28.)).child(
                    div()
                        .w(px(720.))
                        .max_w_full()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .opacity(0.3 + 0.7 * eased)
                        .ml(px(14. * (1. - eased)))
                        .child(
                            div()
                                .text_size(px(30.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.foreground)
                                .child(title),
                        )
                        .child(div().pb(px(10.)).text_size(px(15.)).text_color(theme.muted).child(sub))
                        .child(body),
                ),
            )
            .child(
                // Lined up with the step above: same width, same edges.
                div()
                    .w(px(720.))
                    .max_w_full()
                    .mx_auto()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pb(px(36.))
                    .child(
                        div()
                            .id("back")
                            .h(px(36.))
                            .ml(px(-14.))
                            .px(px(14.))
                            .flex()
                            .items_center()
                            .rounded(px(ui::R_CONTROL + 2.))
                            .text_size(px(ui::T_LG))
                            .text_color(theme.muted)
                            .cursor_pointer()
                            .hover(|d| d.bg(theme.hairline).text_color(theme.foreground))
                            .child("Back")
                            .active(|s| s.opacity(0.7))
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| this.back(&WelcomeBack, window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .children((0..count).map(|i| div().size(px(6.)).rounded_full().bg(dot(i)))),
                    )
                    .child(
                        Self::primary("next", if last { "Start coding" } else { "Continue" }, &theme)
                            .active(|s| s.opacity(0.7))
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| this.next(&WelcomeNext, window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// The typefaces, each shown in itself, the size, and a preview.
    fn type_step(&self, settings: &Settings, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let installed = cx.text_system().all_font_names();
        let fonts: Vec<(&'static str, &'static str)> = crate::fonts::CODE_FONTS
            .iter()
            .copied()
            .filter(|(family, _)| installed.iter().any(|i| i == family))
            .take(9)
            .collect();
        let step_size = |delta: f32| {
            cx.listener(move |_, _: &ClickEvent, _, cx| {
                settings::update(cx, |s| {
                    s.font_size = (s.font_size + delta).clamp(settings::MIN_FONT_SIZE, settings::MAX_FONT_SIZE)
                })
            })
        };
        let stepper = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .size(px(ui::CONTROL))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(ui::R_CONTROL))
                .border_1()
                .border_color(theme.line_strong)
                .text_color(theme.foreground)
                .cursor_pointer()
                .hover(|d| d.bg(theme.hairline))
                .child(label)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().grid().grid_cols(3).gap(px(8.)).children(fonts.into_iter().enumerate().map(
                |(i, (family, label))| {
                    let active = settings.code_font == family;
                    div()
                        .id(("font", i))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .px(px(14.))
                        .py(px(9.))
                        .rounded(px(ui::R_POPOVER))
                        .border_1()
                        .font_family(SharedString::from(family))
                        .cursor_pointer()
                        .when(active, |d| d.border_color(theme.caret).bg(theme.accent_soft))
                        .when(!active, |d| d.border_color(theme.line_strong).hover(|d| d.bg(theme.hairline)))
                        .child(div().text_size(px(ui::T_LG)).text_color(theme.foreground).child(label))
                        .child(div().text_size(px(ui::T_SM)).text_color(theme.muted).child("{ 0O 1lI } => ;"))
                        .active(|s| s.opacity(0.7))
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            settings::update(cx, |s| s.code_font = family.to_string())
                        }))
                },
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .text_size(px(ui::T_MD))
                    .text_color(theme.muted)
                    .child("Size")
                    .child(stepper("smaller", "−").active(|s| s.opacity(0.7)).on_click(step_size(-1.)))
                    .child(
                        div()
                            .w(px(48.))
                            .text_center()
                            .text_color(theme.foreground)
                            .child(format!("{:.0} px", settings.font_size)),
                    )
                    .child(stepper("bigger", "+").active(|s| s.opacity(0.7)).on_click(step_size(1.))),
            )
            .child(Self::type_preview(settings, theme, cx.global::<crate::fonts::Fonts>().code.clone()))
            .into_any_element()
    }

    /// Providers in three groups, as compact cards; then what the one picked needs.
    fn provider_step(&self, settings: &Settings, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let current = settings.ai.provider;
        let group =
            |title: &str, items: &[(ProviderId, &'static str)], needs: Option<AnyElement>, cx: &mut Context<Self>| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(ui::section_heading(title, theme))
                    .children(Some(div().grid().grid_cols(3).gap(px(8.)).children(items.iter().map(|&(id, line)| {
                        let active = current == id;
                        div()
                            .id(id.label())
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .px(px(14.))
                            .py(px(9.))
                            .rounded(px(ui::R_POPOVER))
                            .border_1()
                            .cursor_pointer()
                            .when(active, |d| d.border_color(theme.caret).bg(theme.accent_soft))
                            .when(!active, |d| d.border_color(theme.line_strong).hover(|d| d.bg(theme.hairline)))
                            .child(div().text_size(px(ui::T_LG)).text_color(theme.foreground).child(id.label()))
                            .child(div().text_size(px(ui::T_SM)).text_color(theme.muted).child(line))
                            .active(|s| s.opacity(0.7))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.key.update(cx, |input, cx| input.set_text("", cx));
                                settings::update(cx, |s| {
                                    s.ai.enabled = true;
                                    s.ai.provider = id;
                                })
                            }))
                    }))))
                    // What the one picked needs, right under its group.
                    .children(needs)
            };
        // What the provider picked needs: its key, or its command installed.
        let needs: Option<AnyElement> = if current.uses_api_key() {
            let found = ai::known_key(current) == Some(true);
            let get = current.key_url().map(|u| format!("Get one at {u}")).unwrap_or_default();
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .h(px(ui::FIELD + 6.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .rounded(px(ui::R_CONTROL + 2.))
                            .border_1()
                            .border_color(theme.line_strong)
                            .bg(theme.sunken)
                            .text_size(px(ui::T_MD))
                            .child(div().w_full().overflow_hidden().child(self.key.clone())),
                    )
                    .child(div().text_size(px(ui::T_SM)).text_color(theme.muted).child(if found {
                        format!("A key is already set. {get}. Kept in the keychain, never in a file.")
                    } else {
                        format!("{get}. Kept in the keychain, never in a file.")
                    }))
                    .into_any_element(),
            )
        } else if matches!(current, ProviderId::ClaudeCode | ProviderId::Codex) {
            let program = if current == ProviderId::ClaudeCode { "claude" } else { "codex" };
            let text = match ai::find_cli(program) {
                Some(_) => format!("The {program} command is installed: you're set."),
                None => ai::install_hint(current).unwrap_or_default().to_string(),
            };
            Some(div().text_size(px(ui::T_SM)).text_color(theme.muted).child(text).into_any_element())
        } else if current == ProviderId::Ollama {
            Some(
                div()
                    .text_size(px(ui::T_SM))
                    .text_color(theme.muted)
                    .child("Install Ollama from ollama.com and pull a model; Null finds it on this computer.")
                    .into_any_element(),
            )
        } else {
            None
        };
        let in_group = |items: &[(ProviderId, &'static str)]| items.iter().any(|(id, _)| *id == current);
        let (mut for_plans, mut for_keys, mut for_local) = (None, None, None);
        if in_group(&SUBSCRIPTIONS) {
            for_plans = needs;
        } else if in_group(&API_PROVIDERS) {
            for_keys = needs;
        } else if in_group(&LOCAL) {
            for_local = needs;
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group("With your subscription", &SUBSCRIPTIONS, for_plans, cx))
            .child(group("With an API key", &API_PROVIDERS, for_keys, cx))
            .child(group("On this computer", &LOCAL, for_local, cx))
            .into_any_element()
    }
}

impl Focusable for Welcome {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Welcome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let content = match self.stage {
            Stage::Hello => self.render_hello(window, cx),
            Stage::Step(n) => self.render_step(n, window, cx),
        };
        div()
            .key_context("Welcome")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::back))
            .size_full()
            .bg(theme.background)
            .font_family(cx.global::<crate::fonts::Fonts>().ui.clone())
            .text_color(theme.foreground)
            .child(content)
    }
}
