//! The Settings window (⌘,): every setting with a real control, in a few sections.
//! The JSON file stays one click away for anything not shown here.

use crate::ai::{self, ProviderId};
use crate::keymap::Keymap;
use crate::palette::Category;
use crate::settings::{self, DEFAULT_FONT_SIZE, Settings};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::{Theme, ThemeName};
use crate::ui;
use gpui::{
    Action, AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    SharedString, Subscription, Window, actions, div, prelude::*, px,
};

actions!(settings_panel, [CloseSettings]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseSettings, Some("SettingsPanel"))]);
}

const UI_FONTS: &[(&str, &str)] =
    &[("Instrument Sans", "Instrument Sans"), (".SystemUIFont", "System"), ("Inter", "Inter")];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
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

/// "Also ⌘+ and ⌘−": a phrase followed by the keys of some actions.
fn keys_line(lead: &str, actions: &[&dyn Action], cx: &App) -> String {
    let keys: Vec<String> = actions.iter().filter_map(|a| crate::palette::shortcut(*a, cx)).collect();
    if keys.is_empty() { String::new() } else { format!("{lead} {}", keys.join(" and ")) }
}

/// The key for an action, or nothing when it has none.
fn key(action: &dyn Action, cx: &App) -> String {
    crate::palette::shortcut(action, cx).unwrap_or_default()
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
    /// Opens on another section than the first.
    pub fn show_section(&mut self, section: Section) {
        self.section = section;
    }

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
                // The words keep a readable width whatever the control beside them.
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .flex_1()
                    .min_w(px(200.))
                    .child(div().text_size(px(ui::T_LG)).text_color(theme.foreground).child(title.to_string()))
                    .children(
                        detail.map(|d| div().text_size(px(ui::T_SM)).text_color(theme.muted).child(d.to_string())),
                    ),
            )
            .child(div().flex_shrink_0().max_w(gpui::relative(0.6)).child(control))
            .into_any_element()
    }

    /// A row whose control needs the width (a model field and its suggestions): the
    /// words above, the control under them, full width.
    fn stacked_row(title: &str, detail: Option<&str>, control: impl IntoElement, theme: &Theme) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .py(px(12.))
            .border_b_1()
            .border_color(theme.hairline)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(ui::T_LG)).text_color(theme.foreground).child(title.to_string()))
                    .children(
                        detail.map(|d| div().text_size(px(ui::T_SM)).text_color(theme.muted).child(d.to_string())),
                    ),
            )
            .child(control)
            .into_any_element()
    }

    /// A text field as wide as its row.
    fn wide_field(input: &Entity<TextInput>, theme: &Theme) -> AnyElement {
        div()
            .w_full()
            .h(px(ui::FIELD + 4.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(ui::R_CONTROL))
            .border_1()
            .border_color(theme.line_strong)
            .bg(theme.background)
            .text_size(px(ui::T_MD))
            .child(div().w_full().overflow_hidden().child(input.clone()))
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
            .cursor_pointer()
            .child(ui::switch(on, theme))
            .active(|s| s.opacity(0.7))
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
        ui::segmented(theme)
            .flex_wrap()
            .children(options.into_iter().enumerate().map(|(i, (value, label))| {
                let active = value == current;
                let pick = pick.clone();
                ui::segment(active, theme)
                    .id((id, i))
                    .cursor_pointer()
                    .when(!active, |d| d.hover(|d| d.text_color(theme.foreground)))
                    .child(label)
                    .active(|s| s.opacity(0.7))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| pick(this, value, cx)))
            }))
            .into_any_element()
    }

    fn button(id: impl Into<gpui::ElementId>, label: &str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px(px(12.))
            .h(px(ui::CONTROL))
            .flex()
            .items_center()
            .rounded(px(ui::R_CONTROL))
            .border_1()
            .border_color(theme.line_strong)
            .text_size(px(ui::T_MD))
            .text_color(theme.foreground)
            .cursor_pointer()
            .hover(|d| d.bg(theme.hairline))
            .child(label.to_string())
    }

    fn heading(text: &str, theme: &Theme) -> AnyElement {
        ui::section_heading(text, theme).pt(px(20.)).pb(px(4.)).into_any_element()
    }

    // ---------- sections ----------

    /// A small preview of a theme: its background with a few lines of colored "code".
    fn theme_card(&self, name: ThemeName, current: ThemeName, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        ui::theme_preview(name, name == current, &theme)
            .id(name.label())
            .cursor_pointer()
            .active(|s| s.opacity(0.7))
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.theme = name)))
            .into_any_element()
    }

    fn appearance(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = cx.global::<Settings>().clone();
        let theme = cx.global::<Theme>().clone();
        let cards: Vec<AnyElement> =
            ThemeName::ALL.into_iter().map(|name| self.theme_card(name, s.theme, cx)).collect();
        let size = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                Self::button("smaller", "−", &theme)
                    .active(|s| s.opacity(0.7))
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
                    .active(|s| s.opacity(0.7))
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.font_size += 1.))),
            )
            .when(s.font_size != DEFAULT_FONT_SIZE, |d| {
                d.child(Self::button("actual-size", "Reset", &theme).active(|s| s.opacity(0.7)).on_click(
                    cx.listener(|_, _: &ClickEvent, _, cx| settings::update(cx, |s| s.font_size = DEFAULT_FONT_SIZE)),
                ))
            });
        let installed = |name: &str| name == ".SystemUIFont" || self.installed_fonts.iter().any(|f| f == name);
        let mut code_fonts: Vec<(String, String)> = crate::fonts::CODE_FONTS
            .iter()
            .filter(|(f, _)| installed(f))
            .map(|(f, l)| (f.to_string(), l.to_string()))
            .collect();
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
            div().grid().grid_cols(3).gap(px(16.)).py(px(12.)).children(cards).into_any_element(),
            Self::heading("Text", &theme),
            Self::row(
                "Text size",
                Some(&keys_line(
                    "Also",
                    &[&crate::workspace::IncreaseFontSize, &crate::workspace::DecreaseFontSize],
                    cx,
                )),
                size,
                &theme,
            ),
            Self::stacked_row(
                "Code font",
                Some("Five come with Null; fonts you install show up here too"),
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
            Self::row(
                "Ligatures",
                Some("Join characters like -> and != into one symbol, if the code font has them"),
                Self::toggle("ligatures", s.ligatures, &theme, cx, |s| s.ligatures = !s.ligatures),
                &theme,
            ),
            Self::heading("Window", &theme),
            Self::row(
                "Show the sidebar",
                Some(&keys_line("Shown and hidden with", &[&crate::workspace::ToggleSidebar], cx)),
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
                Some(&format!(
                    "Fit lines to the window instead of scrolling sideways. {}",
                    key(&crate::menus::ToggleWordWrap, cx)
                )),
                Self::toggle("wrap", s.word_wrap, &theme, cx, |s| s.word_wrap = !s.word_wrap),
                &theme,
            ),
            Self::row(
                "Indentation",
                Some("For new files, and files that don't show their own. Files keep theirs, and .editorconfig wins"),
                {
                    use crate::file_style::Indent;
                    Self::choices(
                        "indent",
                        vec![
                            (Indent::Spaces(2), "2 spaces".into()),
                            (Indent::Spaces(4), "4 spaces".into()),
                            (Indent::Tabs, "Tabs".into()),
                        ],
                        s.default_indent(),
                        &theme,
                        cx,
                        |_, indent, cx| {
                            settings::update(cx, |s| match indent {
                                Indent::Tabs => s.indent_with_tabs = true,
                                Indent::Spaces(n) => {
                                    s.indent_with_tabs = false;
                                    s.indent_size = n;
                                }
                            })
                        },
                    )
                },
                &theme,
            ),
            Self::heading("Around the code", &theme),
            Self::row(
                "Indent guides",
                Some("Faint lines down the indentation, to see what's inside what"),
                Self::toggle("indent-guides", s.indent_guides, &theme, cx, |s| s.indent_guides = !s.indent_guides),
                &theme,
            ),
            Self::row(
                "Sticky scroll",
                Some("Inside a long function or block, its first line stays at the top"),
                Self::toggle("sticky-scroll", s.sticky_scroll, &theme, cx, |s| s.sticky_scroll = !s.sticky_scroll),
                &theme,
            ),
            Self::row(
                "Other uses of a name",
                Some("With the caret on a name, its other uses in the file get a soft tint"),
                Self::toggle("symbol-marks", s.symbol_marks, &theme, cx, |s| s.symbol_marks = !s.symbol_marks),
                &theme,
            ),
            Self::row(
                "Who changed the line",
                Some("At the end of the caret's line, faintly: who last changed it, when, and why. From git"),
                Self::toggle("line-blame", s.line_blame, &theme, cx, |s| s.line_blame = !s.line_blame),
                &theme,
            ),
            Self::heading("Saving", &theme),
            Self::row(
                "Save automatically",
                Some("Files with a name save by themselves; ⌘S still formats first when that's on"),
                {
                    use crate::settings::AutoSave;
                    Self::choices(
                        "auto-save",
                        vec![
                            (AutoSave::Off, "Off".into()),
                            (AutoSave::AfterPause, "After a pause".into()),
                            (AutoSave::WhenLeaving, "When leaving the file".into()),
                        ],
                        s.auto_save,
                        &theme,
                        cx,
                        |_, mode, cx| settings::update(cx, |s| s.auto_save = mode),
                    )
                },
                &theme,
            ),
            Self::row(
                "Format on save",
                Some(&format!(
                    "Tidies the file with its language server when saving. {} formats any time",
                    key(&crate::editor::FormatDocument, cx)
                )),
                Self::toggle("format-on-save", s.format_on_save, &theme, cx, |s| s.format_on_save = !s.format_on_save),
                &theme,
            ),
            Self::heading("Code intelligence", &theme),
            Self::row(
                "Suggestions while typing",
                Some(&format!(
                    "From the language server. {} asks for them either way",
                    key(&crate::editor::ShowCompletions, cx)
                )),
                Self::toggle("autocomplete", s.autocomplete, &theme, cx, |s| s.autocomplete = !s.autocomplete),
                &theme,
            ),
            Self::row(
                "Type hints",
                Some("Types and parameter names inside the code, faintly, from the language server"),
                Self::toggle("inlay-hints", s.inlay_hints, &theme, cx, |s| s.inlay_hints = !s.inlay_hints),
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
            .gap(px(6.))
            .children(recommended.iter().enumerate().map(|(i, r)| {
                let active = current.as_deref() == Some(r.model);
                let input = input.clone();
                div()
                    .id((id, i))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(ui::CONTROL))
                    .px(px(10.))
                    .rounded(px(ui::R_CONTROL))
                    .border_1()
                    .text_size(px(ui::T_SM))
                    .cursor_pointer()
                    .when(active, |d| d.border_color(theme.caret).bg(theme.accent_soft).text_color(theme.foreground))
                    .when(!active, |d| {
                        d.border_color(theme.line_strong)
                            .text_color(theme.muted)
                            .hover(|d| d.text_color(theme.foreground).bg(theme.hairline))
                    })
                    .child(r.model)
                    .child(div().text_color(theme.faint).child(r.note))
                    .active(|s| s.opacity(0.7))
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
                    div().text_size(px(ui::T_MD)).text_color(theme.caret).child("Installing…").into_any_element(),
                ),
                (_, Some(path)) => {
                    // Where it is, briefly: "with Xcode", "~/.cargo/bin", "installed by Null".
                    let home = std::env::var("HOME").unwrap_or_default();
                    let full = path.display().to_string();
                    let shown = if full.contains("/Xcode.app/") || full.starts_with("/Library/Developer/") {
                        "with Xcode".to_string()
                    } else if crate::tools::data_dir().is_some_and(|d| path.starts_with(d)) {
                        "installed by Null".to_string()
                    } else {
                        path.parent().map_or(full.clone(), |d| d.display().to_string()).replacen(&home, "~", 1)
                    };
                    (
                        format!("{} · {shown}", server.name),
                        div().text_size(px(ui::T_MD)).text_color(theme.muted).child("Installed").into_any_element(),
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
                            .active(|s| s.opacity(0.7))
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
                            .active(|s| s.opacity(0.7))
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
                &format!("The {program} command"),
                Some(&detail),
                div().text_size(px(ui::T_MD)).text_color(theme.muted).child(status),
                &theme,
            ));
        }
        if current.uses_api_key() {
            let get = current.key_url().map(|u| format!(" · get one at {u}")).unwrap_or_default();
            let (status, label) = match self.has_key {
                Some(true) => (format!("Saved in the keychain{get}"), "Change…"),
                Some(false) => (format!("Not set yet{get}"), "Set API key…"),
                // Not read yet this session (reading it can ask for permission).
                None => (format!("Kept in the system keychain{get}"), "Set API key…"),
            };
            rows.push(Self::row(
                "API key",
                Some(&status),
                Self::button("api-key", label, &theme).active(|s| s.opacity(0.7)).on_click(cx.listener(
                    |_, _: &ClickEvent, _, cx| cx.emit(SettingsPanelEvent::Run(Box::new(crate::workspace::SetApiKey))),
                )),
                &theme,
            ));
        }
        let model = s.ai.model(current);
        rows.push(Self::stacked_row(
            "Model",
            Some(if matches!(current, ProviderId::ClaudeCode | ProviderId::Codex) {
                "For edits, answers and tasks. Empty: the one chosen in the tool"
            } else {
                "For edits, answers and tasks. Empty: the first one below"
            }),
            div().flex().flex_col().gap(px(8.)).child(Self::wide_field(&self.model, &theme)).child(Self::model_chips(
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
            rows.push(Self::stacked_row(
                "Address",
                Some("Where the service answers, if not the usual place"),
                Self::wide_field(&self.address, &theme),
                &theme,
            ));
        }
        rows.push(Self::heading("Suggestions while typing", &theme));
        rows.push(Self::row(
            "Suggest code as you type",
            Some(&format!(
                "Names from the file at once, then the AI's guess when you pause. {} takes it, {} a word, {} a line, {} another",
                key(&crate::editor::AcceptGhost, cx),
                key(&crate::editor::AcceptGhostWord, cx),
                key(&crate::editor::AcceptGhostLine, cx),
                key(&crate::editor::NextGhost, cx),
            )),
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
                rows.push(Self::stacked_row(
                    "Model for suggestions",
                    Some("A code model that fills in the middle is best: fast, and it continues your code"),
                    div().flex().flex_col().gap(px(8.)).child(Self::wide_field(&self.completion_model, &theme)).child(
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
            Some(&format!(
                "{} edits the code at the caret; a question there, or “Ask About This File” in {}, gets a note",
                key(&crate::editor::InlineAssist, cx),
                key(&crate::workspace::ShowCommands, cx),
            )),
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
            Self::stacked_row(
                "Keys from",
                Some(current.summary()),
                Self::choices("keymap", presets, current, &theme, cx, |_, keymap, cx| {
                    settings::update(cx, |s| s.keymap = keymap)
                }),
                &theme,
            ),
        ];
        // From the keymap, not the window: Settings has focus, and the window only knows the
        // keys of what's focused (editor shortcuts would be missing).
        let _ = window;
        let keys_of = |s: &Shortcut| crate::palette::shortcut(s.action.as_ref(), cx);
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
                        .child(ui::key_cap(keys, &theme))
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
                    .active(|s| s.opacity(0.7))
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
                    .active(|s| s.opacity(0.7))
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
            .h(px(560.))
            // Short windows: it shrinks and scrolls rather than losing its bottom.
            .max_h(gpui::relative(0.86))
            .max_w_full()
            .flex()
            .overflow_hidden()
            .rounded(px(ui::R_MODAL))
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
