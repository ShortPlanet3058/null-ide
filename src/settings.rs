use crate::fonts::{self, DEFAULT_CODE_FONT, DEFAULT_UI_FONT};
use crate::theme::{Theme, ThemeName};
use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_FONT_SIZE: f32 = 14.;
pub const MIN_FONT_SIZE: f32 = 9.;
pub const MAX_FONT_SIZE: f32 = 32.;

/// What a language may set for itself, and that a toggle flips for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PerLanguage {
    WordWrap,
    FormatOnSave,
    Autocomplete,
}

/// A language's own settings (see `Settings::languages`): each one given wins over the
/// general one; one left out follows it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguageSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_with_tabs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_on_save: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_on_paste: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_wrap: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autocomplete: Option<bool>,
}

/// How the settings file read last time: why it couldn't be at all, or the keys it had
/// that couldn't be (kept as written when saving).
#[derive(Clone, Debug, Default)]
struct FileState {
    path: Option<PathBuf>,
    unreadable: Option<String>,
    skipped: Vec<String>,
}

static FILE: std::sync::Mutex<FileState> =
    std::sync::Mutex::new(FileState { path: None, unreadable: None, skipped: Vec::new() });

/// Everything Null remembers between launches. Stored as JSON so it can be
/// edited by hand; missing or unknown fields fall back to defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeName,
    /// Follow the Mac's light and dark: `theme` in dark mode, `light_theme` in light mode.
    pub match_appearance: bool,
    pub light_theme: ThemeName,
    /// A theme of your own (`themes/<name>.json`), shown in `theme`'s place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_theme: Option<String>,
    /// Font family for code. Any installed font works; Geist Mono ships with Null.
    pub code_font: String,
    /// Font family for menus, tabs and the palette. Instrument Sans ships with Null.
    pub ui_font: String,
    pub font_size: f32,
    /// Room between lines of code.
    pub line_spacing: LineSpacing,
    /// Where the line numbers show.
    pub line_numbers: LineNumbers,
    /// Where spaces and tabs show, as faint dots and dashes.
    pub whitespace: ShowWhitespace,
    pub sidebar_visible: bool,
    /// Dim the title bar, sidebar and status bar while typing.
    pub fade_bars_while_typing: bool,
    /// Wrap long lines to the width of the editor instead of scrolling sideways.
    pub word_wrap: bool,
    /// Markdown and plain text wrap on their own: they're paragraphs, not code.
    pub wrap_prose: bool,
    /// Wrapped lines break at the project's line length (its guide) when narrower than the
    /// window, rather than at the window's edge.
    pub wrap_at_guide: bool,
    /// Indentation for files that don't show their own (and have no .editorconfig):
    /// this many spaces, or tabs when `indent_with_tabs`.
    pub indent_size: usize,
    pub indent_with_tabs: bool,
    /// Let the code font join characters like -> or != into one symbol.
    pub ligatures: bool,
    /// Format the file with its language server when saving with ⌘S.
    pub format_on_save: bool,
    /// Pasted code formatted by the language server (where it formats parts of a file).
    pub format_on_paste: bool,
    /// A file clicked once in the files opens in a passing tab, which the next one replaces
    /// until it's edited or double-clicked.
    pub preview_tabs: bool,
    /// Brackets coloured by how deep they are, a pair the same colour.
    pub bracket_colours: bool,
    /// Every line's problem written at its end, faintly (otherwise only the caret's line's).
    pub problems_at_line_ends: bool,
    /// Save files without ⌘S: never, after a pause in typing, or when leaving them.
    pub auto_save: AutoSave,
    /// Quitting (or closing the window) with unsaved changes keeps them for next time,
    /// without asking.
    pub keep_unsaved: bool,
    /// Whose shortcuts to use: Null's own, or another editor's.
    pub keymap: crate::keymap::Keymap,
    /// Your own shortcuts over those: a key and the command it runs, or null for none.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub keys: std::collections::BTreeMap<String, Option<String>>,
    /// Your own tasks for ⌘⇧B, in every project: a name and the command it runs.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tasks: std::collections::BTreeMap<String, String>,
    /// Settings for a language over these, by its name as ⌘K's "Language:" lists it:
    /// `"Go": { "indent_with_tabs": true }`.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub languages: std::collections::BTreeMap<String, LanguageSettings>,
    /// Set once the first-launch welcome has been seen.
    pub welcomed: bool,
    /// At the end of the caret's line, faintly: who last changed it, when, and why.
    pub line_blame: bool,
    /// Faint lines down the indentation, one per level.
    pub indent_guides: bool,
    /// The caret blinks for a while once typing stops; off, it stays lit.
    pub caret_blink: bool,
    /// The line being written stays in the middle of the window (typewriter scrolling).
    pub typewriter: bool,
    /// In Markdown and text, the paragraph being written stands out, the others fade.
    pub dim_paragraphs: bool,
    /// In Markdown and text, typed quotes curl and two hyphens make a dash.
    pub smart_punctuation: bool,
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
            match_appearance: false,
            light_theme: ThemeName::Paper,
            own_theme: None,
            code_font: DEFAULT_CODE_FONT.into(),
            ui_font: DEFAULT_UI_FONT.into(),
            font_size: DEFAULT_FONT_SIZE,
            line_spacing: LineSpacing::Normal,
            line_numbers: LineNumbers::Shown,
            whitespace: ShowWhitespace::Selection,
            sidebar_visible: true,
            fade_bars_while_typing: false,
            word_wrap: false,
            wrap_prose: true,
            wrap_at_guide: false,
            indent_size: 4,
            indent_with_tabs: false,
            ligatures: true,
            format_on_save: false,
            format_on_paste: false,
            preview_tabs: true,
            bracket_colours: false,
            problems_at_line_ends: false,
            auto_save: AutoSave::Off,
            keep_unsaved: true,
            keymap: Default::default(),
            keys: Default::default(),
            tasks: Default::default(),
            languages: Default::default(),
            welcomed: false,
            autocomplete: true,
            line_blame: true,
            inlay_hints: false,
            indent_guides: true,
            caret_blink: true,
            typewriter: false,
            dim_paragraphs: false,
            smart_punctuation: false,
            spell_check: true,
            line_guide: true,
            sticky_scroll: true,
            symbol_marks: true,
            ai: Default::default(),
        }
    }
}

impl Global for Settings {}

/// Where line numbers show: everywhere, in code only (not in Markdown and text), or nowhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineNumbers {
    #[default]
    Shown,
    InCode,
    Hidden,
}

/// Where spaces and tabs show: in the selection, also at the ends of lines (where they're
/// left by mistake), or everywhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowWhitespace {
    #[default]
    Selection,
    Trailing,
    All,
}

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

    /// What's set for `language` (its name, in any case), if anything.
    fn language(&self, language: &str) -> Option<&LanguageSettings> {
        self.languages.iter().find(|(name, _)| name.eq_ignore_ascii_case(language)).map(|(_, set)| set)
    }

    /// The indentation for a new `language` file (one that doesn't show its own).
    pub fn indent_for(&self, language: &str) -> crate::file_style::Indent {
        let set = self.language(language);
        // Go and Make are written with tabs, unless said otherwise for them.
        let tabs_by_nature = matches!(language, "Go" | "Makefile");
        let tabs = set.and_then(|s| s.indent_with_tabs).unwrap_or(tabs_by_nature || self.indent_with_tabs);
        let size = set.and_then(|s| s.indent_size).unwrap_or(self.indent_size);
        if tabs { crate::file_style::Indent::Tabs } else { crate::file_style::Indent::Spaces(size.clamp(1, 16)) }
    }

    /// Flips `which` for `language`: its own value, when it has one (so the toggle
    /// changes what that file does), else the general one.
    pub fn flip(&mut self, which: PerLanguage, language: &str) {
        let own =
            self.languages.iter_mut().find(|(name, _)| name.eq_ignore_ascii_case(language)).and_then(|(_, set)| {
                match which {
                    PerLanguage::WordWrap => set.word_wrap.as_mut(),
                    PerLanguage::FormatOnSave => set.format_on_save.as_mut(),
                    PerLanguage::Autocomplete => set.autocomplete.as_mut(),
                }
            });
        let value = match own {
            Some(value) => value,
            None => match which {
                PerLanguage::WordWrap => &mut self.word_wrap,
                PerLanguage::FormatOnSave => &mut self.format_on_save,
                PerLanguage::Autocomplete => &mut self.autocomplete,
            },
        };
        *value = !*value;
    }

    pub fn format_on_save_for(&self, language: &str) -> bool {
        self.language(language).and_then(|s| s.format_on_save).unwrap_or(self.format_on_save)
    }

    pub fn format_on_paste_for(&self, language: &str) -> bool {
        self.language(language).and_then(|s| s.format_on_paste).unwrap_or(self.format_on_paste)
    }

    /// Whether long lines wrap in `language` code (prose goes by `wrap_prose`).
    pub fn word_wrap_for(&self, language: &str) -> bool {
        self.language(language).and_then(|s| s.word_wrap).unwrap_or(self.word_wrap)
    }

    pub fn autocomplete_for(&self, language: &str) -> bool {
        self.language(language).and_then(|s| s.autocomplete).unwrap_or(self.autocomplete)
    }

    /// `~/.config/null/settings.json` on macOS and Linux (or `$XDG_CONFIG_HOME/null`),
    /// `%APPDATA%\Null\settings.json` on Windows.
    pub fn path() -> Option<PathBuf> {
        // Tests never read or write the real settings.
        if cfg!(test) {
            let dir = crate::tools::test_dir("test-config");
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

    /// The settings in the file: None when it can't be read as JSON at all (then nothing
    /// is saved over it until it can: see `FILE`).
    fn load() -> Option<Self> {
        match Self::path() {
            Some(path) => Self::load_from(&path),
            None => Some(Self::default()),
        }
    }

    fn load_from(path: &std::path::Path) -> Option<Self> {
        let path = path.to_path_buf();
        let Ok(text) = std::fs::read_to_string(&path) else {
            *FILE.lock().unwrap_or_else(|e| e.into_inner()) = FileState { path: Some(path), ..Default::default() };
            return Some(Self::default());
        };
        // Comments and trailing commas, as other editors' settings allow.
        match Self::parse_lenient(&crate::snippets::without_comments(&text)) {
            Ok((settings, skipped)) => {
                if !skipped.is_empty() {
                    eprintln!("null: {}: kept the defaults for {}", path.display(), skipped.join(", "));
                }
                *FILE.lock().unwrap_or_else(|e| e.into_inner()) =
                    FileState { path: Some(path), unreadable: None, skipped };
                Some(settings)
            }
            // Not JSON at all: left as it is, and not written over, until it's put right.
            Err(err) => {
                eprintln!("null: can't read {}: {err}", path.display());
                std::fs::copy(&path, path.with_extension("unreadable.json")).ok();
                *FILE.lock().unwrap_or_else(|e| e.into_inner()) =
                    FileState { path: Some(path), unreadable: Some(err.to_string()), skipped: Vec::new() };
                None
            }
        }
    }

    /// Why the settings file can't be read now, if it can't (nothing is saved over it).
    pub fn unreadable() -> Option<String> {
        FILE.lock().unwrap_or_else(|e| e.into_inner()).unreadable.clone()
    }

    /// The settings as written, each one that can't be read (a typo in a theme's name)
    /// falling back to its default alone: one mistake doesn't cost the others. The keys
    /// left at their defaults are returned too.
    fn parse_lenient(text: &str) -> serde_json::Result<(Self, Vec<String>)> {
        if let Ok(settings) = Self::parse(text) {
            return Ok((settings, Vec::new()));
        }
        let written: serde_json::Value = serde_json::from_str(text)?;
        let mut merged = serde_json::to_value(Self::default())?;
        let mut skipped = Vec::new();
        for (key, value) in written.as_object().into_iter().flatten() {
            let mut attempt = merged.clone();
            attempt[key] = value.clone();
            if serde_json::from_value::<Self>(attempt.clone()).is_ok() {
                merged = attempt;
            } else {
                skipped.push(key.clone());
            }
        }
        let mut settings: Self = serde_json::from_value(merged)?;
        settings.font_size = settings.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        Ok((settings, skipped))
    }

    fn parse(text: &str) -> serde_json::Result<Self> {
        let mut settings: Self = serde_json::from_str(text)?;
        settings.font_size = settings.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        Ok(settings)
    }

    /// Writes the settings, into the file as it's written: keys this Null doesn't know (a
    /// newer one's), and values it couldn't read (left at their defaults here), are kept.
    /// Never over a file that can't be read: that waits until it's put right.
    fn save(&self) {
        if let Some(path) = Self::path() {
            self.save_to(&path);
        }
    }

    fn save_to(&self, path: &std::path::Path) {
        let path = path.to_path_buf();
        // What's known of this file (another's says nothing about it).
        let state = Some(FILE.lock().unwrap_or_else(|e| e.into_inner()).clone())
            .filter(|s| s.path.as_ref() == Some(&path))
            .unwrap_or_default();
        if state.unreadable.is_some() {
            eprintln!("null: not saving settings over {}, which can't be read", path.display());
            return;
        }
        let mut out = serde_json::to_value(self).unwrap_or_default();
        let defaults = serde_json::to_value(Self::default()).unwrap_or_default();
        let written: Option<serde_json::Value> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&crate::snippets::without_comments(&text)).ok());
        if let (Some(serde_json::Value::Object(written)), Some(out)) = (written, out.as_object_mut()) {
            for (key, value) in written {
                let unknown = !out.contains_key(&key);
                // Couldn't be read, and not changed since: as written.
                let unread = state.skipped.contains(&key) && out.get(&key) == defaults.get(&key);
                if unknown || unread {
                    out.insert(key, value);
                }
            }
        }
        let result = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| {
            crate::fs_ops::write_file(&path, (serde_json::to_string_pretty(&out).unwrap_or_default() + "\n").as_bytes())
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

impl Settings {
    /// Picks `name`: as the light mode's theme when following the Mac's appearance and it's
    /// a light one, otherwise as the theme.
    pub fn pick_theme(&mut self, name: ThemeName) {
        if self.match_appearance && name.is_light() {
            self.light_theme = name;
        } else {
            self.theme = name;
            self.own_theme = None;
        }
    }

    /// Whether the Mac's light appearance is the one followed now.
    fn light_now(&self, cx: &App) -> bool {
        self.match_appearance
            && matches!(cx.window_appearance(), gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight)
    }

    /// The theme to show now: following the Mac's appearance, the one for it.
    pub fn shown_theme(&self, cx: &App) -> ThemeName {
        if self.light_now(cx) { self.light_theme } else { self.theme }
    }

    /// The theme of your own shown now, if one is.
    pub fn shown_own_theme(&self, cx: &App) -> Option<&str> {
        self.own_theme.as_deref().filter(|_| !self.light_now(cx))
    }

    /// The colours to show now: a theme of your own (if its file reads), or Null's.
    pub fn theme_now(&self, cx: &App) -> Theme {
        self.shown_own_theme(cx)
            .and_then(|name| crate::theme::own::load(name).ok())
            .map_or_else(|| Theme::named(self.shown_theme(cx)), |(theme, _)| theme)
    }
}

/// The Mac switched between light and dark: the theme follows, when it's asked to.
pub fn appearance_changed(cx: &mut App) {
    // Rare (the Mac switched): the theme set again whole, so one of yours that shares a
    // background with Null's is swapped as well.
    let theme = cx.global::<Settings>().theme_now(cx);
    cx.set_global(theme);
    cx.refresh_windows();
}

/// A theme file of your own was saved: its colours now.
pub fn reapply_theme(cx: &mut App) {
    let theme = cx.global::<Settings>().theme_now(cx);
    cx.set_global(theme);
    cx.refresh_windows();
}

pub fn init(cx: &mut App) {
    fonts::register(cx);
    let settings = Settings::load().unwrap_or_default();
    cx.set_global(settings.theme_now(cx));
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
    // Can't be read: the settings stay as they were (see `Settings::unreadable`).
    let Some(settings) = Settings::load() else { return };
    if settings != *cx.global::<Settings>() {
        apply(settings, cx);
    }
}

fn apply(settings: Settings, cx: &mut App) {
    let old = cx.global::<Settings>();
    let theme_changed =
        settings.shown_theme(cx) != old.shown_theme(cx) || settings.shown_own_theme(cx) != old.shown_own_theme(cx);
    let fonts_changed = settings.code_font != old.code_font || settings.ui_font != old.ui_font;
    let keymap = (settings.keymap != old.keymap || settings.keys != old.keys).then_some(settings.keymap);
    if theme_changed {
        cx.set_global(settings.theme_now(cx));
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

    /// A language's own settings win; what it leaves out follows the general ones.
    #[test]
    fn a_language_has_its_own() {
        let settings = Settings::parse(
            r#"{ "indent_size": 4, "format_on_save": true,
                 "languages": { "python": { "indent_size": 2, "word_wrap": true }, "Go": { "indent_with_tabs": true, "format_on_save": false } } }"#,
        )
        .unwrap();
        use crate::file_style::Indent;
        assert_eq!(settings.indent_for("Python"), Indent::Spaces(2), "named in any case");
        assert_eq!(settings.indent_for("Go"), Indent::Tabs);
        assert_eq!(settings.indent_for("Rust"), Indent::Spaces(4));
        assert!(settings.word_wrap_for("Python") && !settings.word_wrap_for("Rust"));
        assert!(!settings.format_on_save_for("Go") && settings.format_on_save_for("Python"));
        // A toggle flips the language's own value where it has one, the general one elsewhere.
        let mut flipped = settings.clone();
        flipped.flip(PerLanguage::WordWrap, "Python");
        assert!(!flipped.word_wrap_for("Python") && !flipped.word_wrap);
        flipped.flip(PerLanguage::WordWrap, "Rust");
        assert!(flipped.word_wrap && !flipped.word_wrap_for("Python"));
        // Go writes with tabs, unless said otherwise for it.
        assert_eq!(Settings::default().indent_for("Go"), Indent::Tabs);
        // Written back as given, nothing added.
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains(r#""python":{"indent_size":2,"word_wrap":true}"#), "{json}");
    }

    /// Saving writes into the file as it's written: keys this Null doesn't know, and values
    /// it couldn't read, stay; comments are allowed; a file that can't be read isn't
    /// written over.
    #[test]
    fn the_file_as_written_is_kept() {
        let dir = crate::tools::test_dir("settings-kept");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            "{\n  // mine\n  \"theme\": \"neon\",\n  \"font_size\": 15,\n  \"from_the_future\": [1, 2],\n}\n",
        )
        .unwrap();
        let mut settings = Settings::load_from(&path).expect("read, comments and all");
        assert_eq!(settings.font_size, 15.);
        settings.word_wrap = !settings.word_wrap;
        settings.save_to(&path);
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["from_the_future"], serde_json::json!([1, 2]), "unknown, kept");
        assert_eq!(saved["theme"], "neon", "couldn't be read, kept as written");
        assert_eq!(saved["word_wrap"], settings.word_wrap);
        // Broken: not read, and not written over.
        std::fs::write(&path, "{ \"font_size\": 15 \"oops\" }").unwrap();
        assert!(Settings::load_from(&path).is_none());
        Settings::default().save_to(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ \"font_size\": 15 \"oops\" }");
        std::fs::remove_dir_all(&dir).ok();
    }

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
    fn one_bad_setting_costs_only_itself() {
        let (s, skipped) =
            Settings::parse_lenient(r#"{ "font_size": 17, "light_theme": "solarized", "caret_blink": false }"#)
                .unwrap();
        assert_eq!((s.font_size, s.caret_blink, s.light_theme), (17., false, ThemeName::Paper));
        assert_eq!(skipped, ["light_theme"]);
        assert!(Settings::parse_lenient("{ not json").is_err());
    }

    #[test]
    fn following_the_macs_appearance_keeps_a_theme_for_each() {
        let mut s = Settings::default();
        s.pick_theme(ThemeName::Paper);
        assert_eq!((s.theme, s.light_theme), (ThemeName::Paper, ThemeName::Paper), "not following: the theme");
        s.theme = ThemeName::Midnight;
        s.match_appearance = true;
        s.pick_theme(ThemeName::Dune);
        s.pick_theme(ThemeName::Ash);
        assert_eq!((s.theme, s.light_theme), (ThemeName::Ash, ThemeName::Dune));
        assert!(!Settings::parse("{}").unwrap().match_appearance);
    }

    #[gpui::test]
    fn the_theme_shown_follows_the_appearance(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let s = Settings { match_appearance: true, light_theme: ThemeName::Dune, ..Settings::default() };
            let light =
                matches!(cx.window_appearance(), gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight);
            assert_eq!(s.shown_theme(cx), if light { ThemeName::Dune } else { ThemeName::Null });
            let off = Settings { match_appearance: false, ..s };
            assert_eq!(off.shown_theme(cx), ThemeName::Null);
        });
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
