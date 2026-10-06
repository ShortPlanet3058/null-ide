use crate::fonts::{self, DEFAULT_CODE_FONT, DEFAULT_UI_FONT};
use crate::theme::{Theme, ThemeName};
use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_FONT_SIZE: f32 = 14.;
pub const MIN_FONT_SIZE: f32 = 9.;
pub const MAX_FONT_SIZE: f32 = 32.;

/// Everything Null remembers between launches. Stored as JSON so it can be
/// edited by hand; missing or unknown fields fall back to defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeName,
    /// Font family for code. Any installed font works; Geist Mono ships with Null.
    pub code_font: String,
    /// Font family for menus, tabs and the palette. Instrument Sans ships with Null.
    pub ui_font: String,
    pub font_size: f32,
    /// Room between lines of code.
    pub line_spacing: LineSpacing,
    pub sidebar_visible: bool,
    /// Dim the title bar, sidebar and status bar while typing.
    pub fade_bars_while_typing: bool,
    /// Wrap long lines to the width of the editor instead of scrolling sideways.
    pub word_wrap: bool,
    /// Markdown and plain text wrap on their own: they're paragraphs, not code.
    pub wrap_prose: bool,
    /// Indentation for files that don't show their own (and have no .editorconfig):
    /// this many spaces, or tabs when `indent_with_tabs`.
    pub indent_size: usize,
    pub indent_with_tabs: bool,
    /// Let the code font join characters like -> or != into one symbol.
    pub ligatures: bool,
    /// Format the file with its language server when saving with ⌘S.
    pub format_on_save: bool,
    /// Save files without ⌘S: never, after a pause in typing, or when leaving them.
    pub auto_save: AutoSave,
    /// Whose shortcuts to use: Null's own, or another editor's.
    pub keymap: crate::keymap::Keymap,
    /// Set once the first-launch welcome has been seen.
    pub welcomed: bool,
    /// At the end of the caret's line, faintly: who last changed it, when, and why.
    pub line_blame: bool,
    /// Faint lines down the indentation, one per level.
    pub indent_guides: bool,
    /// The caret blinks for a while once typing stops; off, it stays lit.
    pub caret_blink: bool,
    /// Misspelled words in Markdown, text and comments get a faint wavy line.
    pub spell_check: bool,
    /// A faint line at the length the project keeps lines to, when it sets one.
    pub line_guide: bool,
    /// Keep the first lines of the blocks scrolled into pinned at the top.
    pub sticky_scroll: bool,
    /// Tint the other uses of the name at the caret.
    pub symbol_marks: bool,
    /// Type hints from the language server inside the code (`x: i32`), faintly.
    pub inlay_hints: bool,
    /// Show suggestions while typing. Ctrl+Space asks for them either way.
    pub autocomplete: bool,
    /// Where AI answers come from. Off until a provider is chosen.
    pub ai: crate::ai::AiSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeName::Null,
            code_font: DEFAULT_CODE_FONT.into(),
            ui_font: DEFAULT_UI_FONT.into(),
            font_size: DEFAULT_FONT_SIZE,
            line_spacing: LineSpacing::Normal,
            sidebar_visible: true,
            fade_bars_while_typing: false,
            word_wrap: false,
            wrap_prose: true,
            indent_size: 4,
            indent_with_tabs: false,
            ligatures: true,
            format_on_save: false,
            auto_save: AutoSave::Off,
            keymap: Default::default(),
            welcomed: false,
            autocomplete: true,
            line_blame: true,
            inlay_hints: false,
            indent_guides: true,
            caret_blink: true,
            spell_check: true,
            line_guide: true,
            sticky_scroll: true,
            symbol_marks: true,
            ai: Default::default(),
        }
    }
}

impl Global for Settings {}

/// How much room there is between lines of code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineSpacing {
    Compact,
    #[default]
    Normal,
    Relaxed,
}

impl LineSpacing {
    pub const ALL: [LineSpacing; 3] = [LineSpacing::Compact, LineSpacing::Normal, LineSpacing::Relaxed];

    pub fn label(self) -> &'static str {
        match self {
            LineSpacing::Compact => "Compact",
            LineSpacing::Normal => "Normal",
            LineSpacing::Relaxed => "Relaxed",
        }
    }

    /// The next one (or the one before), going round.
    pub fn step(self, step: isize) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(1) as isize;
        Self::ALL[(i + step).rem_euclid(Self::ALL.len() as isize) as usize]
    }

    /// A line's height, as a multiple of the text's size.
    pub fn factor(self) -> f32 {
        match self {
            LineSpacing::Compact => 1.45,
            LineSpacing::Normal => 1.7,
            LineSpacing::Relaxed => 2.0,
        }
    }
}

/// When files save by themselves. Only files with a name: a new one waits for ⌘S.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoSave {
    #[default]
    Off,
    /// A second after typing stops.
    AfterPause,
    /// On switching to another tab, or to another app.
    WhenLeaving,
}

impl AutoSave {
    /// How long typing has to stop before saving, in [`AutoSave::AfterPause`].
    pub const PAUSE: std::time::Duration = std::time::Duration::from_millis(1000);
}

impl Settings {
    /// The indentation new files get, and files that don't show their own.
    pub fn default_indent(&self) -> crate::file_style::Indent {
        if self.indent_with_tabs {
            crate::file_style::Indent::Tabs
        } else {
            crate::file_style::Indent::Spaces(self.indent_size.clamp(1, 16))
        }
    }

    /// `~/.config/null/settings.json` on macOS and Linux (or `$XDG_CONFIG_HOME/null`),
    /// `%APPDATA%\Null\settings.json` on Windows.
    pub fn path() -> Option<PathBuf> {
        // Tests never read or write the real settings.
        if cfg!(test) {
            let dir = std::env::temp_dir().join(format!("null-test-config-{}", std::process::id()));
            return Some(dir.join("settings.json"));
        }
        let dir = if cfg!(target_os = "windows") {
            PathBuf::from(std::env::var_os("APPDATA")?).join("Null")
        } else if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            PathBuf::from(xdg).join("null")
        } else {
            PathBuf::from(std::env::var_os("HOME")?).join(".config").join("null")
        };
        Some(dir.join("settings.json"))
    }

    fn load() -> Self {
        let Some(path) = Self::path() else { return Self::default() };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|err| {
                eprintln!("null: ignoring {}: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    fn parse(text: &str) -> serde_json::Result<Self> {
        let mut settings: Self = serde_json::from_str(text)?;
        settings.font_size = settings.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        Ok(settings)
    }

    fn save(&self) {
        let Some(path) = Self::path() else { return };
        let result = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| {
            crate::fs_ops::write_file(&path, (serde_json::to_string_pretty(self).unwrap_or_default() + "\n").as_bytes())
        });
        if let Err(err) = result {
            eprintln!("null: couldn't save settings to {}: {err}", path.display());
        }
    }

    /// Makes sure the settings file exists, so it can be opened for editing.
    pub fn ensure_file(cx: &App) -> Option<PathBuf> {
        let path = Self::path()?;
        if !path.exists() {
            cx.global::<Self>().save();
        }
        Some(path)
    }
}

pub fn init(cx: &mut App) {
    fonts::register(cx);
    let settings = Settings::load();
    cx.set_global(Theme::named(settings.theme));
    fonts::apply(&settings.code_font, &settings.ui_font, cx);
    cx.set_global(settings);
}

/// Changes settings, saves them, and applies the theme.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Settings)) {
    let mut settings = cx.global::<Settings>().clone();
    change(&mut settings);
    settings.font_size = settings.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
    if settings != *cx.global::<Settings>() {
        settings.save();
        apply(settings, cx);
    }
}

/// Re-reads the settings file, after it was edited by hand.
pub fn reload(cx: &mut App) {
    let settings = Settings::load();
    if settings != *cx.global::<Settings>() {
        apply(settings, cx);
    }
}

fn apply(settings: Settings, cx: &mut App) {
    let old = cx.global::<Settings>();
    let theme_changed = settings.theme != old.theme;
    let fonts_changed = settings.code_font != old.code_font || settings.ui_font != old.ui_font;
    let keymap = (settings.keymap != old.keymap).then_some(settings.keymap);
    if theme_changed {
        cx.set_global(Theme::named(settings.theme));
    }
    if fonts_changed {
        fonts::apply(&settings.code_font, &settings.ui_font, cx);
    }
    cx.set_global(settings);
    // New keys right away; the menus show shortcuts, so they're rebuilt too.
    if let Some(keymap) = keymap {
        crate::keymap::register(keymap, cx);
        crate::menus::set(cx);
    }
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let settings = Settings::parse(r#"{ "theme": "paper" }"#).unwrap();
        assert_eq!(settings.theme, ThemeName::Paper);
        assert_eq!(settings.font_size, DEFAULT_FONT_SIZE);
        assert!(settings.sidebar_visible);
    }

    #[test]
    fn font_size_is_clamped_and_unknown_fields_ignored() {
        let settings = Settings::parse(r#"{ "font_size": 400, "something_new": 1 }"#).unwrap();
        assert_eq!(settings.font_size, MAX_FONT_SIZE);
    }

    #[test]
    fn line_spacing_is_saved_by_name_and_normal_unless_set() {
        assert_eq!(Settings::default().line_spacing, LineSpacing::Normal);
        let settings = Settings::parse(r#"{ "line_spacing": "relaxed" }"#).unwrap();
        assert_eq!(settings.line_spacing.factor(), 2.0);
    }

    #[test]
    fn the_caret_blinks_unless_told_not_to() {
        assert!(Settings::default().caret_blink);
        assert!(!Settings::parse(r#"{ "caret_blink": false }"#).unwrap().caret_blink);
    }

    #[test]
    fn round_trips_through_json() {
        let settings = Settings { theme: ThemeName::Ash, font_size: 16., ..Default::default() };
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(Settings::parse(&text).unwrap(), settings);
    }
}
