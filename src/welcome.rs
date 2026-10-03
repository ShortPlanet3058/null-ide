//! The first launch: "hello" in one language after another, then four short steps (how
//! Null looks, the typeface, the shortcuts, AI), each applied as soon as it's picked.
//! Shown again any time with "Welcome to Null…".

use crate::ai::ProviderId;
use crate::keymap::Keymap;
use crate::settings::{self, Settings};
use crate::theme::{Syntax, Theme, ThemeName};
use crate::ui;
use gpui::{
    AnyElement, App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla, KeyBinding,
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
const STEPS: usize = 4;

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

/// What AI the person picked; "not now" switches AI off entirely.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AiChoice {
    NotNow,
    Provider(ProviderId),
}

const AI_CHOICES: [(AiChoice, &str, &str); 5] = [
    (AiChoice::NotNow, "Not now", "AI stays out of sight. Turn it on any time in Settings."),
    (AiChoice::Provider(ProviderId::ClaudeCode), "Claude Code", "Your Claude plan, through the claude command."),
    (AiChoice::Provider(ProviderId::Codex), "Codex", "Your ChatGPT plan, through the codex command."),
    (AiChoice::Provider(ProviderId::Nvidia), "A free API key", "NVIDIA's free models; paste the key in Settings."),
    (AiChoice::Provider(ProviderId::Ollama), "Ollama", "Models running on this computer."),
];

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
            _ticker: ticker,
        }
    }

    fn go(&mut self, stage: Stage, cx: &mut Context<Self>) {
        self.stage = stage;
        self.step_since = Instant::now();
        cx.notify();
    }

    fn next(&mut self, _: &WelcomeNext, _: &mut Window, cx: &mut Context<Self>) {
        match self.stage {
            Stage::Hello => self.go(Stage::Step(0), cx),
            Stage::Step(n) if n + 1 < STEPS => self.go(Stage::Step(n + 1), cx),
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
        let blink_on = (self.hello_since.elapsed().as_millis() / 550) % 2 == 0;
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
                        Self::primary("begin", "Begin", &theme).on_click(
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
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.finish(cx))),
                    ),
            )
            .into_any_element()
    }

    /// The top: room for the window buttons (and dragging the window), the wordmark, and
    /// how far along the setup is.
    fn top_bar(step: Option<usize>, theme: &Theme) -> AnyElement {
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
            .children(
                step.map(|n| {
                    div().text_size(px(ui::T_SM)).text_color(theme.faint).child(format!("{} of {STEPS}", n + 1))
                }),
            )
            .into_any_element()
    }

    fn render_step(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let settings = cx.global::<Settings>().clone();
        let theme = cx.global::<Theme>().clone();
        let t = (self.step_since.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.);
        if t < 1. {
            window.request_animation_frame();
        }
        let eased = 1. - (1. - t).powi(3);
        let (title, sub, body): (&str, &str, AnyElement) = match n {
            0 => (
                "Choose how Null looks",
                "Switch any time from ⌘K or Settings.",
                div()
                    .flex()
                    .gap(px(22.))
                    .children([ThemeName::Oled, ThemeName::Graphite, ThemeName::Paper].into_iter().map(|name| {
                        ui::theme_preview_scaled(name, settings.theme == name, &theme, 1.55)
                            .id(name.label())
                            .cursor_pointer()
                            .on_click(
                                cx.listener(move |_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.theme = name)),
                            )
                    }))
                    .into_any_element(),
            ),
            1 => {
                let installed = cx.text_system().all_font_names();
                let fonts: Vec<&str> = crate::settings_panel::CODE_FONTS
                    .iter()
                    .copied()
                    .filter(|f| installed.iter().any(|i| i == f))
                    .take(4)
                    .collect();
                let size = settings.font_size;
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
                (
                    "Pick a typeface you can read all day",
                    "Monospaced and made for code. The size changes with ⌘+ and ⌘- too.",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child(div().flex().flex_wrap().gap(px(10.)).children(fonts.into_iter().enumerate().map(
                            |(i, font)| {
                                let active = settings.code_font == font;
                                let name = font.to_string();
                                div()
                                    .id(("font", i))
                                    .px(px(14.))
                                    .py(px(8.))
                                    .rounded(px(ui::R_CONTROL + 2.))
                                    .border_1()
                                    .font_family(SharedString::from(font))
                                    .text_size(px(ui::T_LG))
                                    .cursor_pointer()
                                    .when(active, |d| {
                                        d.border_color(theme.caret).bg(theme.accent_soft).text_color(theme.foreground)
                                    })
                                    .when(!active, |d| {
                                        d.border_color(theme.line_strong)
                                            .text_color(theme.muted)
                                            .hover(|d| d.text_color(theme.foreground))
                                    })
                                    .child(font)
                                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                        let name = name.clone();
                                        settings::update(cx, |s| s.code_font = name)
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
                                .child(stepper("smaller", "−").on_click(step_size(-1.)))
                                .child(
                                    div()
                                        .w(px(48.))
                                        .text_center()
                                        .text_color(theme.foreground)
                                        .child(format!("{size:.0} px")),
                                )
                                .child(stepper("bigger", "+").on_click(step_size(1.))),
                        )
                        .child(Self::type_preview(&settings, &theme, cx.global::<crate::fonts::Fonts>().code.clone()))
                        .into_any_element(),
                )
            }
            2 => (
                "Which shortcuts do your hands know?",
                "Null's own keep the common VS Code ones. Change them any time in Settings.",
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(10.))
                    .children(Keymap::ALL.into_iter().enumerate().map(|(i, k)| {
                        Self::card(("keymap", i), k.label(), k.summary(), settings.keymap == k, &theme).on_click(
                            cx.listener(move |_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.keymap = k)),
                        )
                    }))
                    .into_any_element(),
            ),
            _ => {
                let now = if settings.ai.enabled && settings.ai.provider != ProviderId::Off {
                    AiChoice::Provider(settings.ai.provider)
                } else {
                    AiChoice::NotNow
                };
                (
                    "How present should AI be?",
                    "It never opens a panel on its own, and never changes code without showing you the change.",
                    div()
                        .grid()
                        .grid_cols(2)
                        .gap(px(10.))
                        .children(AI_CHOICES.into_iter().enumerate().map(|(i, (choice, title, line))| {
                            Self::card(("ai", i), title, line, now == choice, &theme).on_click(cx.listener(
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
                        }))
                        .into_any_element(),
                )
            }
        };
        let last = n + 1 == STEPS;
        let dot = |i: usize| -> Hsla { if i == n { theme.caret } else { theme.line_strong } };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(Self::top_bar(Some(n), &theme))
            .child(
                div().flex_1().min_h_0().flex().justify_center().items_center().px(px(28.)).child(
                    div()
                        .w(px(680.))
                        .max_w_full()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .opacity(0.3 + 0.7 * eased)
                        .ml(px(14. * (1. - eased)))
                        .child(
                            div()
                                .text_size(px(32.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.foreground)
                                .child(title),
                        )
                        .child(div().pb(px(12.)).text_size(px(15.)).text_color(theme.muted).child(sub))
                        .child(body),
                ),
            )
            .child(
                // Lined up with the step above: same width, same edges.
                div()
                    .w(px(680.))
                    .max_w_full()
                    .mx_auto()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pb(px(40.))
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
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| this.back(&WelcomeBack, window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .children((0..STEPS).map(|i| div().size(px(6.)).rounded_full().bg(dot(i)))),
                    )
                    .child(
                        Self::primary("next", if last { "Start coding" } else { "Continue" }, &theme).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.next(&WelcomeNext, window, cx)),
                        ),
                    ),
            )
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
