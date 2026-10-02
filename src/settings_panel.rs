//! The Settings window (⌘,): every setting with a real control, in a few sections.
//! The JSON file stays one click away for anything not shown here.

use crate::ai::{self, ProviderId};
use crate::keymap::Keymap;
use crate::palette::Category;
use crate::settings::{self, DEFAULT_FONT_SIZE, Settings};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::{Syntax, Theme, ThemeName};
use gpui::{
    Action, AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla,
    KeyBinding, SharedString, Subscription, Window, actions, div, prelude::*, px,
};

actions!(settings_panel, [CloseSettings]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseSettings, Some("SettingsPanel"))]);
}

const CODE_FONTS: &[&str] = &["Geist Mono", "SF Mono", "Menlo", "JetBrains Mono", "Fira Code", "Cascadia Code"];
const UI_FONTS: &[(&str, &str)] =
    &[("Instrument Sans", "Instrument Sans"), (".SystemUIFont", "System"), ("Inter", "Inter")];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Appearance,
    Editor,
    Languages,
    Ai,
    Keyboard,
}

impl Section {
    const ALL: [Section; 5] =
        [Section::Appearance, Section::Editor, Section::Languages, Section::Ai, Section::Keyboard];

    fn label(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Editor => "Editor",
            Section::Languages => "Languages",
            Section::Ai => "AI",
            Section::Keyboard => "Keyboard",
        }
    }
}

/// A command to list in the Keyboard section, with whatever key the preset gives it.
pub struct Shortcut {
    pub category: Category,
    pub label: SharedString,
    pub action: Box<dyn Action>,
}

pub enum SettingsPanelEvent {
    Closed,
    /// Run a command after closing, like setting an API key or opening the JSON file.
    Run(Box<dyn Action>),
}

pub struct SettingsPanel {
    focus_handle: FocusHandle,
    lsp: Entity<crate::lsp_store::LspStore>,
    section: Section,
    shortcuts: Vec<Shortcut>,
    installed_fonts: Vec<String>,
    model: Entity<TextInput>,
    address: Entity<TextInput>,
    completion_model: Entity<TextInput>,
    /// The provider the two fields above were filled for.
    fields_for: ProviderId,
    /// Whether that provider's key is set. Checked once, since it reads the keychain.
    has_key: Option<bool>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsPanelEvent> for SettingsPanel {}

impl SettingsPanel {
    pub fn new(shortcuts: Vec<Shortcut>, lsp: Entity<crate::lsp_store::LspStore>, cx: &mut Context<Self>) -> Self {
        let model = cx.new(|cx| TextInput::new("Model", cx));
        let address = cx.new(|cx| TextInput::new("Address", cx));
        let completion_model = cx.new(|cx| TextInput::new("Same as above", cx));
        let subscriptions = vec![
            // Install progress shows as it happens.
            cx.observe(&lsp, |_, _, cx| cx.notify()),
            cx.subscribe(&model, |this, input, TextInputEvent::Changed, cx| {
                let text = input.read(cx).text().trim().to_string();
                let provider = this.fields_for;
                let value = (!text.is_empty()).then_some(text);
                settings::update(cx, |s| {
                    // Don't add an empty entry just for showing the field.
                    if value.is_some() || s.ai.providers.contains_key(provider.key()) {
                        s.ai.providers.entry(provider.key().into()).or_default().model = value;
                    }
                });
            }),
            cx.subscribe(&completion_model, |this, input, TextInputEvent::Changed, cx| {
                let text = input.read(cx).text().trim().to_string();
                let provider = this.fields_for;
                let value = (!text.is_empty()).then_some(text);
                settings::update(cx, |s| {
                    if value.is_some() || s.ai.providers.contains_key(provider.key()) {
                        s.ai.providers.entry(provider.key().into()).or_default().completion_model = value;
                    }
                });
            }),
            cx.subscribe(&address, |this, input, TextInputEvent::Changed, cx| {
                let text = input.read(cx).text().trim().to_string();
                let provider = this.fields_for;
                let value = (!text.is_empty()).then_some(text);
                settings::update(cx, |s| {
                    if value.is_some() || s.ai.providers.contains_key(provider.key()) {
                        s.ai.providers.entry(provider.key().into()).or_default().base_url = value;
                    }
                });
            }),
        ];
        let installed_fonts = cx.text_system().all_font_names();
        let mut panel = Self {
            focus_handle: cx.focus_handle(),
            lsp,
            section: Section::Appearance,
            shortcuts,
            installed_fonts,
            model,
            address,
            completion_model,
            fields_for: ProviderId::Off,
            has_key: None,
            _subscriptions: subscriptions,
        };
        panel.fill_provider_fields(cx);
        panel
    }

    /// Shows the current provider's model and address in the fields.
    fn fill_provider_fields(&mut self, cx: &mut Context<Self>) {
        let ai = cx.global::<Settings>().ai.clone();
        let provider = ai.provider;
        self.fields_for = provider;
        let saved = ai.providers.get(provider.key()).cloned().unwrap_or_default();
        let model = saved.model.unwrap_or_default();
        let address = saved.base_url.unwrap_or_default();
        let model_hint = provider.default_model().map_or("Model".to_string(), |m| format!("Model (default: {m})"));
        let address_hint = provider.default_base_url().map_or("Address".to_string(), |u| format!("Address ({u})"));
        // Filling the fields saves them straight back, unchanged: `fields_for` is already this provider.
        self.model.update(cx, |input, cx| {
            input.set_placeholder(model_hint);
            input.set_text(&model, cx);
        });
        self.address.update(cx, |input, cx| {
            input.set_placeholder(address_hint);
            input.set_text(&address, cx);
        });
        let completion_hint = match ai.completion_model(provider) {
            Some(default) if saved.completion_model.is_none() => format!("Default: {default}"),
            _ => "Same as the model above".to_string(),
        };
        let completion_model = saved.completion_model.clone().unwrap_or_default();
        self.completion_model.update(cx, |input, cx| {
            input.set_placeholder(completion_hint);
            input.set_text(&completion_model, cx);
        });
        // Never read the keychain just to show this: it can make macOS ask for permission.
        self.has_key = provider.uses_api_key().then(|| ai::known_key(provider)).flatten();
    }

    fn close(&mut self, _: &CloseSettings, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsPanelEvent::Closed);
    }

    fn set_provider(&mut self, provider: ProviderId, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.ai.provider = provider);
        self.fill_provider_fields(cx);
        cx.notify();
    }

    // ---------- controls ----------

    /// A setting: its name and a line of explanation on the left, the control on the right.
    fn row(title: &str, detail: Option<&str>, control: impl IntoElement, theme: &Theme) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(24.))
            .py(px(12.))
            .border_b_1()
            .border_color(theme.hairline)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .min_w_0()
                    .child(div().text_size(px(14.)).text_color(theme.foreground).child(title.to_string()))
                    .children(detail.map(|d| div().text_size(px(12.)).text_color(theme.faint).child(d.to_string()))),
            )
            .child(div().flex_none().child(control))
            .into_any_element()
    }

    fn toggle(
        id: &'static str,
        on: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Settings) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .w(px(36.))
            .h(px(20.))
            .p(px(2.))
            .rounded_full()
            .cursor_pointer()
            .flex()
            .when(on, |d| d.justify_end().bg(theme.caret))
            .when(!on, |d| d.bg(theme.hairline))
            .child(div().size(px(16.)).rounded_full().bg(if on { theme.background } else { theme.muted }))
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| settings::update(cx, &change)))
            .into_any_element()
    }

    /// A row of choices, the current one lit.
    fn choices<T: Copy + PartialEq + 'static>(
        id: &'static str,
        options: Vec<(T, String)>,
        current: T,
        theme: &Theme,
        cx: &mut Context<Self>,
        pick: impl Fn(&mut Self, T, &mut Context<Self>) + Clone + 'static,
    ) -> AnyElement {
        div()
            .flex()
            .flex_wrap()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(8.))
            .bg(theme.hairline.opacity(0.5))
            .children(options.into_iter().enumerate().map(|(i, (value, label))| {
                let active = value == current;
                let pick = pick.clone();
                div()
                    .id((id, i))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .cursor_pointer()
                    .when(active, |d| d.bg(theme.raised).text_color(theme.foreground).shadow_sm())
                    .when(!active, |d| d.text_color(theme.muted).hover(|d| d.text_color(theme.foreground)))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| pick(this, value, cx)))
            }))
            .into_any_element()
    }

    fn button(id: impl Into<gpui::ElementId>, label: &str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px(px(12.))
            .py(px(5.))
            .rounded(px(7.))
            .border_1()
            .border_color(theme.hairline)
            .text_size(px(13.))
            .text_color(theme.foreground)
            .cursor_pointer()
            .hover(|d| d.bg(theme.accent_soft))
            .child(label.to_string())
    }

    fn field(input: &Entity<TextInput>, theme: &Theme) -> AnyElement {
        div()
            .w(px(300.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(7.))
            .border_1()
            .border_color(theme.hairline)
            .bg(theme.background)
            .text_size(px(13.))
            .child(input.clone())
            .into_any_element()
    }

    fn heading(text: &str, theme: &Theme) -> AnyElement {
        div()
            .pt(px(18.))
            .pb(px(2.))
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.faint)
            .child(text.to_uppercase())
            .into_any_element()
    }

    // ---------- sections ----------

    /// A small preview of a theme: its background with a few lines of colored "code".
    fn theme_card(&self, name: ThemeName, current: ThemeName, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let preview = Theme::named(name);
        let active = name == current;
        let bar = |w: f32, color: Hsla| div().h(px(5.)).w(px(w)).rounded(px(2.)).bg(color);
        div()
            .id(name.label())
            .flex()
            .flex_col()
            .gap(px(8.))
            .cursor_pointer()
            .child(
                div()
                    .w(px(132.))
                    .h(px(78.))
                    .p(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(7.))
                    .rounded(px(10.))
                    .bg(preview.background)
                    .border_2()
                    .border_color(if active { theme.caret } else { theme.hairline })
                    .child(
                        div()
                            .flex()
                            .gap(px(5.))
                            .child(bar(22., preview.syntax(Syntax::Keyword)))
                            .child(bar(40., preview.syntax(Syntax::Function))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(5.))
                            .pl(px(10.))
                            .child(bar(30., preview.foreground))
                            .child(bar(36., preview.syntax(Syntax::String))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(5.))
                            .pl(px(10.))
                            .child(bar(18., preview.syntax(Syntax::Comment)))
                            .child(bar(2., preview.caret)),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(if active { theme.foreground } else { theme.muted })
                    .child(name.label()),
            )
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.theme = name)))
            .into_any_element()
    }

    fn appearance(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = cx.global::<Settings>().clone();
        let theme = cx.global::<Theme>().clone();
        let cards: Vec<AnyElement> = [ThemeName::Oled, ThemeName::Graphite, ThemeName::Paper]
            .into_iter()
            .map(|name| self.theme_card(name, s.theme, cx))
            .collect();
        let size = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                Self::button("smaller", "−", &theme)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.font_size -= 1.))),
            )
            .child(
                div()
                    .w(px(44.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(theme.foreground)
                    .child(format!("{}", s.font_size)),
            )
            .child(
                Self::button("bigger", "+", &theme)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.font_size += 1.))),
            )
            .when(s.font_size != DEFAULT_FONT_SIZE, |d| {
                d.child(Self::button("actual-size", "Reset", &theme).on_click(
                    cx.listener(|_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.font_size = DEFAULT_FONT_SIZE)),
                ))
            });
        let installed = |name: &str| name == ".SystemUIFont" || self.installed_fonts.iter().any(|f| f == name);
        let mut code_fonts: Vec<(String, String)> =
            CODE_FONTS.iter().filter(|f| installed(f)).map(|f| (f.to_string(), f.to_string())).collect();
        if !code_fonts.iter().any(|(f, _)| *f == s.code_font) {
            code_fonts.push((s.code_font.clone(), s.code_font.clone()));
        }
        let mut ui_fonts: Vec<(String, String)> =
            UI_FONTS.iter().filter(|(f, _)| installed(f)).map(|(f, l)| (f.to_string(), l.to_string())).collect();
        if !ui_fonts.iter().any(|(f, _)| *f == s.ui_font) {
            ui_fonts.push((s.ui_font.clone(), s.ui_font.clone()));
        }
        let code_index = code_fonts.iter().position(|(f, _)| *f == s.code_font).unwrap_or(0);
        let ui_index = ui_fonts.iter().position(|(f, _)| *f == s.ui_font).unwrap_or(0);
        let code_options = code_fonts.iter().enumerate().map(|(i, (_, l))| (i, l.clone())).collect();
        let ui_options = ui_fonts.iter().enumerate().map(|(i, (_, l))| (i, l.clone())).collect();
        vec![
            Self::heading("Theme", &theme),
            div().flex().gap(px(16.)).py(px(12.)).children(cards).into_any_element(),
            Self::heading("Text", &theme),
            Self::row("Text size", Some("Also ⌘+ and ⌘−"), size, &theme),
            Self::row(
                "Code font",
                None,
                Self::choices("code-font", code_options, code_index, &theme, cx, move |_, i, cx| {
                    let font = code_fonts[i].0.clone();
                    settings::update(cx, |s| s.code_font = font);
                }),
                &theme,
            ),
            Self::row(
                "Interface font",
                None,
                Self::choices("ui-font", ui_options, ui_index, &theme, cx, move |_, i, cx| {
                    let font = ui_fonts[i].0.clone();
                    settings::update(cx, |s| s.ui_font = font);
                }),
                &theme,
            ),
            Self::heading("Window", &theme),
            Self::row(
                "Show the sidebar",
                Some("⌘B shows and hides it"),
                Self::toggle("sidebar", s.sidebar_visible, &theme, cx, |s| s.sidebar_visible = !s.sidebar_visible),
                &theme,
            ),
            Self::row(
                "Fade bars while typing",
                Some("Dims the title bar, sidebar and status bar so only the code stands out"),
                Self::toggle("fade", s.fade_bars_while_typing, &theme, cx, |s| {
                    s.fade_bars_while_typing = !s.fade_bars_while_typing
                }),
                &theme,
            ),
        ]
    }

    fn editor(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = cx.global::<Settings>().clone();
        let theme = cx.global::<Theme>().clone();
        vec![
            Self::heading("Text", &theme),
            Self::row(
                "Wrap long lines",
                Some("Fit lines to the window instead of scrolling sideways. ⌥Z"),
                Self::toggle("wrap", s.word_wrap, &theme, cx, |s| s.word_wrap = !s.word_wrap),
                &theme,
            ),
            Self::row(
                "Indentation",
                Some("Tab inserts spaces; Tab and ⇧Tab indent selected lines"),
                div().text_size(px(13.)).text_color(theme.muted).child("4 spaces"),
                &theme,
            ),
            Self::heading("Code intelligence", &theme),
            Self::row(
                "Suggestions while typing",
                Some("From the language server. ⌃Space asks for them either way"),
                Self::toggle("autocomplete", s.autocomplete, &theme, cx, |s| s.autocomplete = !s.autocomplete),
                &theme,
            ),
        ]
    }

    /// Recommended models as chips under a model field; a click fills the field.
    fn model_chips(
        id: &'static str,
        recommended: &'static [ai::Recommended],
        current: Option<String>,
        input: &Entity<TextInput>,
        theme: &Theme,
    ) -> AnyElement {
        div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(px(6.))
            .pt(px(6.))
            .children(recommended.iter().enumerate().map(|(i, r)| {
                let active = current.as_deref() == Some(r.model);
                let input = input.clone();
                div()
                    .id((id, i))
                    .flex()
                    .gap(px(5.))
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(6.))
                    .border_1()
                    .text_size(px(11.))
                    .cursor_pointer()
                    .when(active, |d| d.border_color(theme.caret).text_color(theme.foreground))
                    .when(!active, |d| d.border_color(theme.hairline).text_color(theme.muted))
                    .child(r.model)
                    .child(div().text_color(theme.faint).child(r.note))
                    .on_click(move |_, _, cx| input.update(cx, |input, cx| input.set_text(r.model, cx)))
            }))
            .into_any_element()
    }

    /// Every language Null has a server for: installed or not, and one click to install.
    fn languages(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.global::<Theme>().clone();
        let mut rows = vec![
            Self::heading("Language servers", &theme),
            div()
                .pt(px(6.))
                .text_size(px(12.))
                .text_color(theme.faint)
                .child("Completions, errors, info on hover and go to definition. Null installs them in its own folder, never on the system.")
                .into_any_element(),
        ];
        for (i, server) in crate::servers::SERVERS.iter().enumerate() {
            let state = self.lsp.read(cx).install_state(server.name);
            let found = crate::servers::find(server);
            let (detail, control): (String, AnyElement) = match (state, &found) {
                (Some(Ok(())), _) => (
                    server.name.to_string(),
                    div().text_size(px(13.)).text_color(theme.caret).child("Installing…").into_any_element(),
                ),
                (_, Some(path)) => {
                    let home = std::env::var("HOME").unwrap_or_default();
                    let shown = path.display().to_string().replacen(&home, "~", 1);
                    (
                        format!("{} · {shown}", server.name),
                        div().text_size(px(13.)).text_color(theme.muted).child("Installed").into_any_element(),
                    )
                }
                (failed, None) => {
                    let can = crate::servers::can_install(server);
                    let detail = match (failed, &can) {
                        (Some(Err(reason)), _) => reason.to_string(),
                        (_, Err(reason)) => reason.clone(),
                        _ => format!("{} · not installed", server.name),
                    };
                    let control = if can.is_ok() {
                        Self::button(("install", i), "Install", &theme)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.lsp.update(cx, |lsp, cx| lsp.install_server(server, cx))
                            }))
                            .into_any_element()
                    } else {
                        div().text_size(px(13.)).text_color(theme.faint).child("Not installed").into_any_element()
                    };
                    (detail, control)
                }
            };
            rows.push(Self::row(server.label, Some(&detail), control, &theme));
        }
        rows
    }

    fn ai(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = cx.global::<Settings>().clone();
        let theme = cx.global::<Theme>().clone();
        let current = s.ai.provider;
        let switch = Self::row(
            "Use AI",
            Some("When off, AI shows up nowhere and nothing leaves your machine"),
            Self::toggle("ai-enabled", s.ai.enabled, &theme, cx, |s| s.ai.enabled = !s.ai.enabled),
            &theme,
        );
        if !s.ai.enabled {
            return vec![Self::heading("AI", &theme), switch];
        }
        let groups: [(&str, &[ProviderId]); 3] = [
            ("Your subscriptions", &[ProviderId::ClaudeCode, ProviderId::Codex]),
            (
                "Free to start",
                &[
                    ProviderId::Mistral,
                    ProviderId::Groq,
                    ProviderId::Gemini,
                    ProviderId::OpenRouter,
                    ProviderId::Nvidia,
                    ProviderId::Ollama,
                ],
            ),
            ("Paid per use", &[ProviderId::Claude, ProviderId::OpenaiCompatible]),
        ];
        let mut rows = vec![Self::heading("AI", &theme), switch];
        if current == ProviderId::Off {
            rows.push(
                div()
                    .pt(px(10.))
                    .text_size(px(12.))
                    .text_color(theme.muted)
                    .child("Choose where answers come from.")
                    .into_any_element(),
            );
        }
        for (title, ids) in groups {
            rows.push(Self::heading(title, &theme));
            rows.push(
                // Two columns, so the chosen provider's options stay close.
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .pt(px(4.))
                    .children(ids.iter().map(|&id| {
                        let active = id == current;
                        div()
                            .id(id.key())
                            .w(px(255.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(9.))
                            .cursor_pointer()
                            .when(active, |d| d.bg(theme.accent_soft))
                            .when(!active, |d| d.hover(|d| d.bg(theme.hairline.opacity(0.5))))
                            .child(
                                div()
                                    .size(px(14.))
                                    .flex_none()
                                    .rounded_full()
                                    .border_2()
                                    .border_color(if active { theme.caret } else { theme.faint })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(active, |d| d.child(div().size(px(6.)).rounded_full().bg(theme.caret))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .flex()
                                    .flex_col()
                                    .child(div().text_size(px(14.)).text_color(theme.foreground).child(id.label()))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .line_height(px(15.))
                                            .text_color(theme.faint)
                                            .child(id.description()),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_provider(id, cx)))
                    }))
                    .into_any_element(),
            );
        }
        if current == ProviderId::Off {
            return rows;
        }
        rows.push(Self::heading(&format!("{} options", current.label()), &theme));
        if matches!(current, ProviderId::ClaudeCode | ProviderId::Codex) {
            let program = if current == ProviderId::ClaudeCode { "claude" } else { "codex" };
            let (status, detail) = match ai::find_cli(program) {
                Some(path) => ("Installed".to_string(), path.display().to_string()),
                None => ("Not installed".to_string(), ai::install_hint(current).unwrap_or_default().to_string()),
            };
            rows.push(Self::row(
                &format!("`{program}` command"),
                Some(&detail),
                div().text_size(px(13.)).text_color(theme.muted).child(status),
                &theme,
            ));
        }
        if current.uses_api_key() {
            let get = current.key_url().map(|u| format!(" · get one at {u}")).unwrap_or_default();
            let (status, label) = match self.has_key {
                Some(true) => (format!("Saved in the keychain{get}"), "Change…"),
                Some(false) => (format!("Not set yet{get}"), "Set API Key…"),
                // Not read yet this session (reading it can ask for permission).
                None => (format!("Kept in the system keychain{get}"), "Set API Key…"),
            };
            rows.push(Self::row(
                "API key",
                Some(&status),
                Self::button("api-key", label, &theme).on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                    cx.emit(SettingsPanelEvent::Run(Box::new(crate::workspace::SetApiKey)))
                })),
                &theme,
            ));
        }
        let model = s.ai.model(current);
        rows.push(Self::row(
            "Model",
            Some(if matches!(current, ProviderId::ClaudeCode | ProviderId::Codex) {
                "For edits and answers. Empty: the one chosen in the tool"
            } else {
                "For edits and answers. Empty: the first one below"
            }),
            div().flex().flex_col().items_end().child(Self::field(&self.model, &theme)).child(Self::model_chips(
                "model",
                current.recommended(),
                model,
                &self.model,
                &theme,
            )),
            &theme,
        ));
        if !current.efforts().is_empty() {
            let effort = s.ai.effort(current);
            let options = current.efforts().iter().map(|e| (*e, e.label().to_string())).collect();
            let key = current.key();
            rows.push(Self::row(
                "Thinking",
                Some("More is slower, and better on hard questions. Suggestions always use the least"),
                Self::choices("effort", options, effort, &theme, cx, move |_, effort, cx| {
                    settings::update(cx, |s| s.ai.providers.entry(key.into()).or_default().effort = Some(effort))
                }),
                &theme,
            ));
        }
        if matches!(current, ProviderId::Nvidia | ProviderId::Ollama | ProviderId::OpenaiCompatible) {
            rows.push(Self::row("Address", None, Self::field(&self.address, &theme), &theme));
        }
        rows.push(Self::heading("Suggestions while typing", &theme));
        rows.push(Self::row(
            "Suggest code as you type",
            Some("Names from the file at once, then the AI's guess when you pause. ⇥ takes it, ⌥→ a word, ⌘→ a line, ⌥⇥ another"),
            Self::toggle("ai-completions", s.ai.completions, &theme, cx, |s| s.ai.completions = !s.ai.completions),
            &theme,
        ));
        if s.ai.completions {
            if matches!(current, ProviderId::ClaudeCode | ProviderId::Codex) {
                rows.push(Self::row(
                    "From the AI",
                    Some("Command line tools take seconds to start, too slow for this: names from the file only. A free Mistral key gives the best suggestions"),
                    div(),
                    &theme,
                ));
            } else {
                rows.push(Self::row(
                    "Model for suggestions",
                    Some("A code model that fills in the middle is best: fast, and it continues your code"),
                    div().flex().flex_col().items_end().child(Self::field(&self.completion_model, &theme)).child(
                        Self::model_chips(
                            "suggestion-model",
                            current.recommended_for_suggestions(),
                            s.ai.completion_model(current),
                            &self.completion_model,
                            &theme,
                        ),
                    ),
                    &theme,
                ));
            }
        }
        rows.push(Self::row(
            "Ask and edit",
            Some("⌘I edits the code at the caret; a question there, or “Ask About This File” in ⌘K, gets a note"),
            div(),
            &theme,
        ));
        rows
    }

    fn keyboard(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.global::<Theme>().clone();
        let current = cx.global::<Settings>().keymap;
        let presets = Keymap::ALL.into_iter().map(|k| (k, k.label().to_string())).collect();
        let mut rows = vec![
            Self::heading("Shortcuts", &theme),
            Self::row(
                "Keys from",
                Some(current.summary()),
                Self::choices("keymap", presets, current, &theme, cx, |_, keymap, cx| {
                    settings::update(cx, |s| s.keymap = keymap)
                }),
                &theme,
            ),
        ];
        let keys_of = |s: &Shortcut| {
            window.highest_precedence_binding_for_action(s.action.as_ref()).map(|b| crate::palette::format_keys(&b))
        };
        for category in Category::ALL {
            let members: Vec<(&Shortcut, String)> = self
                .shortcuts
                .iter()
                .filter(|s| s.category == category)
                .filter_map(|s| Some((s, keys_of(s)?)))
                .collect();
            if members.is_empty() {
                continue;
            }
            rows.push(Self::heading(category.label(), &theme));
            for (shortcut, keys) in members {
                rows.push(
                    div()
                        .flex()
                        .justify_between()
                        .py(px(7.))
                        .border_b_1()
                        .border_color(theme.hairline)
                        .text_size(px(13.))
                        .child(div().text_color(theme.foreground).child(shortcut.label.clone()))
                        .child(
                            div()
                                .px(px(6.))
                                .py(px(1.))
                                .rounded(px(5.))
                                .bg(theme.hairline)
                                .text_size(px(12.))
                                .text_color(theme.muted)
                                .child(keys),
                        )
                        .into_any_element(),
                );
            }
        }
        rows
    }
}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The provider can change elsewhere (the palette); keep the fields in step.
        if cx.global::<Settings>().ai.provider != self.fields_for {
            self.fill_provider_fields(cx);
        }
        let theme = cx.global::<Theme>().clone();
        let content = match self.section {
            Section::Appearance => self.appearance(cx),
            Section::Editor => self.editor(cx),
            Section::Languages => self.languages(cx),
            Section::Ai => self.ai(cx),
            Section::Keyboard => self.keyboard(window, cx),
        };
        let current = self.section;
        let sidebar = div()
            .w(px(190.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(12.))
            .border_r_1()
            .border_color(theme.hairline)
            .child(
                div()
                    .px(px(10.))
                    .pt(px(6.))
                    .pb(px(14.))
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child("Settings"),
            )
            .children(Section::ALL.into_iter().map(|section| {
                let active = section == current;
                div()
                    .id(section.label())
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(7.))
                    .text_size(px(14.))
                    .cursor_pointer()
                    .when(active, |d| d.bg(theme.accent_soft).text_color(theme.foreground))
                    .when(!active, |d| d.text_color(theme.muted).hover(|d| d.text_color(theme.foreground)))
                    .child(section.label())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.section = section;
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(
                div()
                    .id("edit-json")
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(7.))
                    .text_size(px(12.))
                    .text_color(theme.faint)
                    .cursor_pointer()
                    .hover(|d| d.text_color(theme.foreground))
                    .child("Edit as JSON…")
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                        cx.emit(SettingsPanelEvent::Run(Box::new(crate::workspace::OpenSettingsFile)))
                    })),
            );
        div()
            .key_context("SettingsPanel")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            // Clicks inside shouldn't reach the backdrop, which closes it.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(780.))
            .h(px(540.))
            .max_w_full()
            .flex()
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(theme.hairline)
            .bg(theme.raised)
            .shadow_lg()
            .child(sidebar)
            .child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .px(px(28.))
                    .pb(px(24.))
                    .flex()
                    .flex_col()
                    .children(content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    /// Every section draws, with and without an AI provider. (Only reads settings: nothing is saved.)
    #[gpui::test]
    fn every_section_renders(cx: &mut TestAppContext) {
        for provider in [ProviderId::Off, ProviderId::Ollama, ProviderId::ClaudeCode] {
            for section in Section::ALL {
                cx.update(|cx| {
                    let mut settings = Settings::default();
                    settings.ai.provider = provider;
                    cx.set_global(settings);
                    cx.set_global(Theme::oled());
                });
                let shortcuts = vec![Shortcut {
                    category: Category::File,
                    label: "Save".into(),
                    action: Box::new(crate::editor::Save),
                }];
                let (panel, cx) = cx.add_window_view(|_, cx| {
                    let lsp = cx.new(|_| crate::lsp_store::LspStore::new(std::path::PathBuf::from("/tmp")));
                    SettingsPanel::new(shortcuts, lsp, cx)
                });
                panel.update(cx, |panel, cx| {
                    panel.section = section;
                    cx.notify();
                });
                cx.run_until_parked();
                panel.read_with(cx, |panel, _| assert!(panel.section == section));
            }
        }
    }
}
