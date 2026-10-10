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

type JsonMap = serde_json::Map<String, serde_json::Value>;

/// The settings of the project in front (`.null/settings.json` in it): over yours while
/// its window is the one in use. `shadowed` is what yours were under them, so they come
/// back, and so saving knows which values are the project's and which are yours.
#[derive(Clone, Debug, Default)]
struct ProjectFile {
    path: Option<PathBuf>,
    values: JsonMap,
    shadowed: JsonMap,
    /// What its own file (`path`) says, of `values` (the rest is from `.vscode`).
    own: JsonMap,
    /// Its own file is there but can't be read (a comma missing mid-edit): never written.
    unreadable: bool,
}

/// The settings that aren't written when empty or unset, and what that is: cleared, they
/// must be written so (or the file's old value would stay).
const UNSET_WHEN_EMPTY: [&str; 4] = ["own_theme", "keys", "tasks", "languages"];

fn unset_value(key: &str) -> serde_json::Value {
    if key == "own_theme" { serde_json::Value::Null } else { serde_json::Value::Object(JsonMap::new()) }
}

/// The key of `map` that's `key` in any case.
fn key_like(map: &JsonMap, key: &str) -> Option<String> {
    map.keys().find(|k| k.eq_ignore_ascii_case(key)).cloned()
}

thread_local! {
    // All on the main thread (in tests, each test's own: one's project isn't another's).
    static PROJECT: std::cell::RefCell<Option<ProjectFile>> = const { std::cell::RefCell::new(None) };
}

fn with_project<R>(f: impl FnOnce(&mut Option<ProjectFile>) -> R) -> R {
    PROJECT.with(|project| f(&mut project.borrow_mut()))
}

/// Where a project keeps its own settings.
pub fn project_file(root: &std::path::Path) -> PathBuf {
    root.join(".null").join("settings.json")
}

/// Where VS Code keeps a project's settings, also read (`.null`'s win over them).
pub fn vscode_file(root: &std::path::Path) -> PathBuf {
    root.join(".vscode").join("settings.json")
}

/// A project's settings as written (comments allowed): none when there's no such file; Err
/// when there is one that can't be read as an object.
fn read_project(path: &std::path::Path) -> Result<JsonMap, ()> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return if path.exists() { Err(()) } else { Ok(JsonMap::new()) };
    };
    serde_json::from_str::<serde_json::Value>(&crate::snippets::without_comments(&text))
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or(())
}

/// A project's `.vscode/settings.json`, in Null's words: the editing settings projects
/// share that way (indentation, formatting, wrapping), for each language too.
fn vscode_settings(root: &std::path::Path) -> JsonMap {
    use serde_json::Value;
    let written = read_project(&vscode_file(root)).unwrap_or_default();
    // The settings a language can have of its own, as VS Code names them.
    let editing = |from: &JsonMap| {
        let mut out = JsonMap::new();
        if let Some(Value::Number(n)) = from.get("editor.tabSize") {
            out.insert("indent_size".into(), Value::Number(n.clone()));
        }
        if let Some(Value::Bool(spaces)) = from.get("editor.insertSpaces") {
            out.insert("indent_with_tabs".into(), Value::Bool(!spaces));
        }
        for (theirs, ours) in [("editor.formatOnSave", "format_on_save"), ("editor.formatOnPaste", "format_on_paste")] {
            if let Some(Value::Bool(on)) = from.get(theirs) {
                out.insert(ours.into(), Value::Bool(*on));
            }
        }
        if let Some(Value::String(wrap)) = from.get("editor.wordWrap") {
            out.insert("word_wrap".into(), Value::Bool(wrap != "off"));
        }
        out
    };
    let mut out = editing(&written);
    if let Some(Value::Bool(on)) = written.get("editor.bracketPairColorization.enabled") {
        out.insert("bracket_colours".into(), Value::Bool(*on));
    }
    let mut languages = JsonMap::new();
    for (key, value) in &written {
        // "[python]", or several at once: "[javascript][typescript]".
        let (Some(ids), Some(set)) = (key.strip_prefix('[').and_then(|k| k.strip_suffix(']')), value.as_object())
        else {
            continue;
        };
        let set = editing(set);
        if set.is_empty() {
            continue;
        }
        for id in ids.split("][") {
            // VS Code's name for it, as Null's (two of VS Code's can be one of Null's).
            let name = match id {
                "typescriptreact" => "TSX",
                "javascript" | "javascriptreact" => "JavaScript",
                "typescript" => "TypeScript",
                "json" | "jsonc" => "JSON",
                "cpp" => "C++",
                "csharp" => "C#",
                "shellscript" => "Shell",
                other => other,
            };
            match key_like(&languages, name).and_then(|k| languages.get_mut(&k)) {
                Some(Value::Object(known)) => known.extend(set.clone()),
                _ => {
                    languages.insert(name.to_string(), Value::Object(set.clone()));
                }
            }
        }
    }
    if !languages.is_empty() {
        out.insert("languages".into(), Value::Object(languages));
    }
    out
}

/// `under` with `over` on top: its languages' settings added to each language's.
fn layered(mut under: JsonMap, over: &JsonMap) -> JsonMap {
    use serde_json::Value;
    for (key, value) in over {
        match (under.get_mut(key), value) {
            (Some(Value::Object(langs)), Value::Object(theirs)) if key == "languages" => {
                for (lang, set) in theirs {
                    let same = langs.keys().find(|k| k.eq_ignore_ascii_case(lang)).cloned();
                    match (same.and_then(|k| langs.get_mut(&k)), set) {
                        (Some(Value::Object(mine)), Value::Object(set)) => mine.extend(set.clone()),
                        _ => {
                            langs.insert(lang.clone(), set.clone());
                        }
                    }
                }
            }
            _ => {
                under.insert(key.clone(), value.clone());
            }
        }
    }
    under
}

/// `user` with `project` over it: each setting it gives wins (one that can't be read is
/// left out); a map of them (`languages`, `keys`) gains its entries instead. With what
/// was under them.
fn merged(user: &Settings, project: &JsonMap) -> (Settings, JsonMap) {
    use serde_json::Value;
    let mut value = serde_json::to_value(user).unwrap_or_default();
    let mut shadowed = JsonMap::new();
    for (key, theirs) in project {
        // Not written when empty or unset (`languages`, `own_theme`): an empty one of yours.
        let mine = value.get(key).cloned().unwrap_or_else(|| match theirs {
            Value::Object(_) => Value::Object(JsonMap::new()),
            _ => Value::Null,
        });
        let mut attempt = value.clone();
        match (&mut attempt[key], theirs) {
            // An entry named in another case ("python", "Python") is the same one.
            (Value::Object(mine), Value::Object(theirs)) => {
                for (entry, value) in theirs {
                    // A language's settings: those it names go over yours, the rest stay.
                    match (key_like(mine, entry), value) {
                        (Some(k), Value::Object(fields)) if mine[&k].is_object() => {
                            if let Some(Value::Object(yours)) = mine.get_mut(&k) {
                                yours.extend(fields.clone());
                            }
                        }
                        (Some(k), _) => {
                            mine.remove(&k);
                            mine.insert(entry.clone(), value.clone());
                        }
                        (None, _) => {
                            mine.insert(entry.clone(), value.clone());
                        }
                    }
                }
            }
            (slot, _) => *slot = theirs.clone(),
        }
        if serde_json::from_value::<Settings>(attempt.clone()).is_ok() {
            value = attempt;
            shadowed.insert(key.clone(), mine);
        }
    }
    let mut settings: Settings = serde_json::from_value(value).unwrap_or_else(|_| user.clone());
    settings.font_size = settings.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
    (settings, shadowed)
}

/// `settings` without the project's over them: yours, as they were under them.
fn unmerged(settings: &Settings, project: &ProjectFile) -> Settings {
    use serde_json::Value;
    let mut value = serde_json::to_value(settings).unwrap_or_default();
    let defaults = serde_json::to_value(Settings::default()).unwrap_or_default();
    for (key, theirs) in &project.values {
        let Some(mine) = project.shadowed.get(key) else { continue };
        match (&mut value[key], theirs, mine) {
            (Value::Object(now), Value::Object(theirs), Value::Object(mine)) => {
                for entry in theirs.keys() {
                    now.retain(|k, _| !k.eq_ignore_ascii_case(entry));
                }
                for (entry, value) in mine {
                    if theirs.keys().any(|t| t.eq_ignore_ascii_case(entry)) {
                        now.insert(entry.clone(), value.clone());
                    }
                }
            }
            (slot, _, _) => *slot = mine.clone(),
        }
    }
    serde_json::from_value(value).unwrap_or_else(|_| serde_json::from_value(defaults).unwrap_or_default())
}

/// `out` (the settings in use, as JSON) parted: the project's own values go to it (the
/// project's settings as they're now, returned), and in `out` yours take their place
/// (also returned, as what's under the project's now).
fn parted(out: &mut JsonMap, project: &ProjectFile) -> (JsonMap, JsonMap) {
    use serde_json::Value;
    let mut theirs_now = project.values.clone();
    let mut shadowed = JsonMap::new();
    for (key, theirs) in &project.values {
        // Only those in use (one that couldn't be read is neither theirs nor yours here).
        let Some(mine) = project.shadowed.get(key) else { continue };
        // Not there: cleared (no theme of your own, no shortcuts left).
        let now = out.get(key).cloned().unwrap_or_else(|| unset_value(key));
        let mine = match (&now, theirs, mine) {
            (Value::Object(now), Value::Object(theirs), Value::Object(mine)) => {
                let mut mine = mine.clone();
                let mut theirs = theirs.clone();
                for (entry, value) in now {
                    match (key_like(&theirs, entry), value) {
                        // A language both set: the fields the project names are its own.
                        (Some(t), Value::Object(fields)) if theirs[&t].is_object() => {
                            // Yours under them as they were, with what was changed of the rest.
                            let mine_key = key_like(&mine, entry).unwrap_or_else(|| entry.clone());
                            let mut yours = mine.get(&mine_key).and_then(Value::as_object).cloned().unwrap_or_default();
                            let mut its = JsonMap::new();
                            for (field, v) in fields {
                                if theirs[&t].get(field).is_some() {
                                    its.insert(field.clone(), v.clone());
                                } else {
                                    yours.insert(field.clone(), v.clone());
                                }
                            }
                            yours.retain(|field, _| fields.contains_key(field) || theirs[&t].get(field).is_some());
                            theirs.insert(t, Value::Object(its));
                            if yours.is_empty() {
                                mine.remove(&mine_key);
                            } else {
                                mine.insert(mine_key, Value::Object(yours));
                            }
                        }
                        (Some(t), _) => {
                            theirs.insert(t, value.clone());
                        }
                        (None, _) => {
                            mine.insert(entry.clone(), value.clone());
                        }
                    }
                }
                // Taken away here: gone from whoever had it.
                theirs.retain(|entry, _| key_like(now, entry).is_some());
                mine.retain(|entry, _| key_like(now, entry).is_some() || key_like(&theirs, entry).is_some());
                theirs_now.insert(key.clone(), Value::Object(theirs));
                Value::Object(mine)
            }
            _ => {
                theirs_now.insert(key.clone(), now);
                mine.clone()
            }
        };
        // Yours: not written when it's only the empty one put under the project's.
        let placeholder = mine.is_null() || mine.as_object().is_some_and(JsonMap::is_empty);
        if placeholder && UNSET_WHEN_EMPTY.contains(&key.as_str()) {
            out.remove(key);
        } else {
            out.insert(key.clone(), mine.clone());
        }
        shadowed.insert(key.clone(), mine);
    }
    (theirs_now, shadowed)
}

/// What goes into a project's own file of `now` (its settings in use): what the file
/// says already, and what isn't as it was (`before`, with .vscode's), down to a
/// language's fields: VS Code's settings aren't copied in.
fn own_part(now: &serde_json::Value, before: Option<&serde_json::Value>, own: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    use serde_json::Value;
    match (now, before) {
        (Value::Object(now), Some(Value::Object(before))) => {
            let mut out = own.and_then(Value::as_object).cloned().unwrap_or_default();
            for (entry, value) in now {
                let was = key_like(before, entry).and_then(|k| before.get(&k));
                let own_key = key_like(&out, entry);
                let own_value = own_key.as_ref().and_then(|k| out.get(k)).cloned();
                if let Some(part) = own_part(value, was, own_value.as_ref()) {
                    out.insert(own_key.unwrap_or_else(|| entry.clone()), part);
                }
            }
            out.retain(|entry, _| key_like(now, entry).is_some());
            (!out.is_empty() || own.is_some()).then_some(Value::Object(out))
        }
        _ => (own.is_some() || before.is_none_or(|b| !same_json(b, now))).then(|| now.clone()),
    }
}

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
    /// Tint the other uses of the name at the caret (and of the text selected).
    pub symbol_marks: bool,
    /// Typing a bracket or quote types its partner too, steps over one already there, and
    /// wraps what's selected.
    pub auto_close: bool,
    /// Type hints from the language server inside the code (`x: i32`), faintly.
    pub inlay_hints: bool,
    /// What the language server says over the caret's function ("3 references", "▶ Run").
    pub code_lens: bool,
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
            code_lens: true,
            indent_guides: true,
            caret_blink: true,
            typewriter: false,
            dim_paragraphs: false,
            smart_punctuation: false,
            spell_check: true,
            line_guide: true,
            sticky_scroll: true,
            symbol_marks: true,
            auto_close: true,
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
                // A copy of it as it was (once: an earlier one may be of settings that read).
                let copy = path.with_extension("unreadable.json");
                if !copy.exists() {
                    std::fs::copy(&path, copy).ok();
                }
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
            let project = with_project(|p| p.clone()).filter(|p| !p.values.is_empty());
            self.save_with(&path, project);
        }
    }

    #[cfg(test)]
    fn save_to(&self, path: &std::path::Path) {
        self.save_with(path, None);
    }

    /// Saves into `path`; the values that are `project`'s (in use over yours) into its file.
    fn save_with(&self, path: &std::path::Path, project: Option<ProjectFile>) {
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
        if let (Some(project), Some(out)) = (project, out.as_object_mut()) {
            let (theirs, shadowed) = parted(out, &project);
            // Into its own file: what it says already, and what changed (one from .vscode
            // too: that file isn't Null's to write).
            let own: JsonMap = theirs
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), own_part(v, project.values.get(k), project.own.get(k))?)))
                .collect();
            // Its file can't be read: not written over (what's yours still goes to yours).
            if own != project.own
                && !project.unreadable
                && let Some(file) = &project.path
            {
                save_project(file, &own);
            }
            with_project(|p| {
                if let Some(p) = p.as_mut()
                    && p.path == project.path
                {
                    p.values = theirs;
                    p.shadowed = shadowed;
                    p.own = own;
                }
            });
        }
        let written: Option<serde_json::Value> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&crate::snippets::without_comments(&text)).ok());
        if let (Some(serde_json::Value::Object(written)), Some(out)) = (written, out.as_object_mut()) {
            // Cleared (no theme of your own, the last shortcut taken away): written so.
            for key in UNSET_WHEN_EMPTY {
                if !out.contains_key(key) && written.contains_key(key) {
                    out.insert(key.to_string(), unset_value(key));
                }
            }
            for (key, value) in written {
                let unknown = !out.contains_key(&key);
                // Couldn't be read, and not changed since: as written.
                let unread = state.skipped.contains(&key) && out.get(&key) == defaults.get(&key);
                if unknown || unread {
                    out.insert(key, value);
                }
            }
        }
        // Into the file as written (its comments, its order), only what changed; else whole.
        let before = std::fs::read_to_string(&path).ok();
        let text = before
            .as_deref()
            .zip(out.as_object())
            .and_then(|(text, out)| written_into(text, out, &defaults))
            .unwrap_or_else(|| serde_json::to_string_pretty(&out).unwrap_or_default() + "\n");
        if before.as_deref() == Some(text.as_str()) {
            return;
        }
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| crate::fs_ops::write_file(&path, text.as_bytes()));
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
    // Couldn't be read: put right since (in another editor)? Read now, so the change goes
    // into what it says, not into the defaults used meanwhile.
    if Settings::unreadable().is_some() {
        reload(cx);
    }
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
    // The project's still over them.
    let settings = with_project(|project| match project.as_mut() {
        Some(project) if !project.values.is_empty() => {
            let (settings, shadowed) = merged(&settings, &project.values);
            project.shadowed = shadowed;
            settings
        }
        _ => settings,
    });
    if settings != *cx.global::<Settings>() {
        apply(settings, cx);
    }
}

/// The project's settings, written into its file as it's written.
fn save_project(file: &std::path::Path, values: &JsonMap) {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let before = std::fs::read_to_string(file).ok();
    let text = match before.as_deref() {
        // Into it as it's written; one this can't follow isn't written over whole.
        Some(text) => match written_into(text, values, &serde_json::Value::Object(JsonMap::new())) {
            Some(text) => text,
            None => return eprintln!("null: not saving over {}, which can't be followed", file.display()),
        },
        None => serde_json::to_string_pretty(values).unwrap_or_default() + "\n",
    };
    if before.as_deref() != Some(text.as_str())
        && let Err(err) = crate::fs_ops::write_file(file, text.as_bytes())
    {
        eprintln!("null: couldn't save {}: {err}", file.display());
    }
}

/// The settings of the project at `root` (its window in front now): over yours, in place
/// of another project's. Read again each time: put right since, they're taken as they are.
pub fn use_project(root: &std::path::Path, cx: &mut App) {
    let path = project_file(root);
    let read = read_project(&path);
    let unreadable = read.is_err();
    let own = read.unwrap_or_default();
    let values = layered(vscode_settings(root), &own);
    let before = with_project(|p| p.take()).unwrap_or_default();
    // Nothing of either's, or the same as in use: nothing changes.
    if before.values.is_empty() && values.is_empty() || before.path.as_ref() == Some(&path) && before.values == values {
        with_project(|p| *p = Some(ProjectFile { path: Some(path), own, unreadable, ..before }));
        return;
    }
    let user = unmerged(cx.global::<Settings>(), &before);
    let (settings, shadowed) = merged(&user, &values);
    with_project(|p| *p = Some(ProjectFile { path: Some(path), values, shadowed, own, unreadable }));
    if settings != *cx.global::<Settings>() {
        apply(settings, cx);
    }
}

/// Whether the project at `root` is the one whose settings are in use.
pub fn project_in_use(root: &std::path::Path) -> bool {
    let path = project_file(root);
    with_project(|p| p.as_ref().is_some_and(|p| p.path.as_ref() == Some(&path)))
}

/// Whether the project in use has a settings file that can't be read.
pub fn project_unreadable() -> bool {
    with_project(|p| p.as_ref().is_some_and(|p| p.unreadable))
}

/// The project's settings as in use now: their names, for saying where a value comes from.
pub fn project_keys() -> Vec<String> {
    with_project(|p| p.as_ref().map(|p| p.values.keys().cloned().collect()).unwrap_or_default())
}

/// `out` written into `text` (settings.json as it is): each value that differs replaced
/// where it is, and those it doesn't have (unless at their defaults) added at its end, so
/// its comments and order stay. None when the text isn't one object this can follow, or
/// doesn't read back as `out`.
fn written_into(
    text: &str,
    out: &serde_json::Map<String, serde_json::Value>,
    defaults: &serde_json::Value,
) -> Option<String> {
    use serde_json::Value;
    let (open, members) = json_members(text)?;
    let indent_at = |at: usize| {
        let line = text[..at].rfind('\n').map_or(0, |n| n + 1);
        text[line..at].chars().take_while(|c| *c == ' ' || *c == '\t').collect::<String>()
    };
    // Nested values (objects, lists) indented as the line their key is on.
    let shown = |value: &Value, indent: &str| {
        serde_json::to_string_pretty(value).ok().map(|v| v.replace('\n', &format!("\n{indent}")))
    };
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for (key, range) in &members {
        let Some(value) = out.get(key) else { continue };
        let written: Option<Value> = serde_json::from_str(&crate::snippets::without_comments(&text[range.clone()])).ok();
        if !written.as_ref().is_some_and(|w| same_json(w, value)) {
            edits.push((range.clone(), shown(value, &indent_at(range.start))?));
        }
    }
    let indent = members.first().map_or_else(|| "  ".to_string(), |(_, r)| indent_at(r.start));
    let mut added = String::new();
    for (key, value) in out {
        if members.iter().any(|(k, _)| k == key) || defaults.get(key) == Some(value) {
            continue;
        }
        if !members.is_empty() || !added.is_empty() {
            added.push(',');
        }
        added.push_str(&format!("\n{indent}{}: {}", serde_json::to_string(key).ok()?, shown(value, &indent)?));
    }
    if !added.is_empty() {
        match members.last() {
            Some((_, last)) => edits.push((last.end..last.end, added)),
            None => edits.push((open + 1..open + 1, added + "\n")),
        }
    }
    let mut new = text.to_string();
    edits.sort_by_key(|(r, _)| r.start);
    for (range, with) in edits.into_iter().rev() {
        new.replace_range(range, &with);
    }
    // Reads back as `out` (a key left out reads as its default), or not used.
    let read: Value = serde_json::from_str(&crate::snippets::without_comments(&new)).ok()?;
    let read = read.as_object()?;
    let same = out.iter().all(|(k, v)| read.get(k).or(defaults.get(k)).is_some_and(|r| same_json(r, v)))
        && read.keys().all(|k| out.contains_key(k));
    same.then_some(new)
}

/// The same JSON, a number the same however it's written (`15` and `15.0`).
fn same_json(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (a, b) {
        // (A value kept as f32, written back, reads as 13.300000190734863 for 13.3.)
        (Value::Number(a), Value::Number(b)) => {
            a.as_f64() == b.as_f64() || ((a.is_f64() || b.is_f64()) && a.as_f64().map(|x| x as f32) == b.as_f64().map(|x| x as f32))
        }
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_json(a, b)),
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| same_json(v, w)))
        }
        _ => a == b,
    }
}

/// A key in a JSON object, and where its value is.
type Member = (String, std::ops::Range<usize>);

/// The keys of the object `text` is (comments and trailing commas allowed) with where each
/// value is, and where the object opens. None when it isn't one object.
fn json_members(text: &str) -> Option<(usize, Vec<Member>)> {
    let b = text.as_bytes();
    let space = |mut i: usize| loop {
        while b.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if b[i.min(b.len())..].starts_with(b"//") {
            while b.get(i).is_some_and(|c| *c != b'\n') {
                i += 1;
            }
        } else if b[i.min(b.len())..].starts_with(b"/*") {
            i = text[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
        } else {
            return i;
        }
    };
    let string_end = |mut i: usize| {
        i += 1;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 2,
                b'"' => return Some(i + 1),
                _ => i += 1,
            }
        }
        None
    };
    let value_end = |mut i: usize| -> Option<usize> {
        match *b.get(i)? {
            b'"' => string_end(i),
            b'{' | b'[' => {
                let mut depth = 0;
                loop {
                    i = space(i);
                    match *b.get(i)? {
                        b'"' => i = string_end(i)?,
                        b'{' | b'[' => {
                            depth += 1;
                            i += 1;
                        }
                        b'}' | b']' => {
                            depth -= 1;
                            i += 1;
                            if depth == 0 {
                                return Some(i);
                            }
                        }
                        _ => i += 1,
                    }
                }
            }
            _ => {
                while b.get(i).is_some_and(|c| !matches!(c, b',' | b'}' | b']' | b'/') && !c.is_ascii_whitespace()) {
                    i += 1;
                }
                Some(i)
            }
        }
    };
    let open = space(0);
    if b.get(open) != Some(&b'{') {
        return None;
    }
    let mut members = Vec::new();
    let mut i = open + 1;
    loop {
        i = space(i);
        match *b.get(i)? {
            b'}' => break,
            b',' => i += 1,
            b'"' => {
                let key_end = string_end(i)?;
                let key: String = serde_json::from_str(&text[i..key_end]).ok()?;
                i = space(key_end);
                if b.get(i) != Some(&b':') {
                    return None;
                }
                let start = space(i + 1);
                let end = value_end(start)?;
                members.push((key, start..end));
                i = end;
            }
            _ => return None,
        }
    }
    (space(i + 1) == b.len()).then_some((open, members))
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

    /// A project's settings are used over yours, its languages added to yours; changed
    /// while in use, its own go back to its file (comments kept) and the rest to yours.
    #[test]
    fn a_project_s_settings_go_over_yours_and_stay_its_own() {
        let dir = crate::tools::test_dir("settings-project");
        let _ = std::fs::remove_dir_all(&dir);
        let project_path = project_file(&dir.join("project"));
        std::fs::create_dir_all(project_path.parent().unwrap()).unwrap();
        std::fs::write(
            &project_path,
            "{\n  // ours\n  \"indent_size\": 2,\n  \"languages\": { \"go\": { \"word_wrap\": true } },\n  \"theme\": \"no such\"\n}\n",
        )
        .unwrap();
        let user_path = dir.join("settings.json");
        let mut user = Settings { word_wrap: true, theme: ThemeName::Paper, ..Settings::default() };
        user.languages.insert("python".into(), LanguageSettings { indent_size: Some(8), ..Default::default() });
        user.save_to(&user_path);
        let values = read_project(&project_path).unwrap();
        let (now, shadowed) = merged(&user, &values);
        assert_eq!(now.indent_size, 2);
        assert!(now.word_wrap, "what it doesn't set is yours");
        assert_eq!(now.theme, user.theme, "one that can't be read is left out");
        assert_eq!(now.languages.keys().collect::<Vec<_>>(), ["go", "python"], "its languages added to yours");
        let project =
            ProjectFile { path: Some(project_path.clone()), own: values.clone(), values, shadowed, unreadable: false };
        assert_eq!(unmerged(&now, &project), user, "yours come back as they were");
        // Changed while in use.
        let mut changed = now.clone();
        changed.indent_size = 3;
        changed.font_size = 17.;
        changed.languages.get_mut("python").unwrap().indent_size = Some(6);
        changed.save_with(&user_path, Some(project));
        let yours = Settings::load_from(&user_path).unwrap();
        assert_eq!((yours.indent_size, yours.font_size), (Settings::default().indent_size, 17.));
        assert_eq!(yours.languages.keys().collect::<Vec<_>>(), ["python"]);
        assert_eq!(yours.languages["python"].indent_size, Some(6));
        assert_eq!(yours.theme, ThemeName::Paper, "not the project's (unreadable) theme, nor the default");
        let theirs = std::fs::read_to_string(&project_path).unwrap();
        assert!(theirs.contains("// ours") && theirs.contains("\"indent_size\": 3"), "{theirs}");
        assert!(theirs.contains("\"go\"") && !theirs.contains("python"), "{theirs}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A project's .vscode/settings.json, as far as Null has the same settings: its
    /// editing ones, for each language too (by VS Code's names for them).
    #[test]
    fn a_project_s_vscode_settings_are_read() {
        let dir = crate::tools::test_dir("settings-vscode");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".vscode")).unwrap();
        std::fs::write(
            vscode_file(&dir),
            r#"{
                // shared with the team
                "editor.tabSize": 2,
                "editor.insertSpaces": true,
                "editor.formatOnSave": true,
                "editor.wordWrap": "bounded",
                "workbench.colorTheme": "Solarized",
                "[python]": { "editor.tabSize": 4 },
                "[typescript][typescriptreact]": { "editor.formatOnSave": false },
                "[markdown]": { "files.trimTrailingWhitespace": false },
            }"#,
        )
        .unwrap();
        let read = vscode_settings(&dir);
        let user = Settings::default();
        let (now, _) = merged(&user, &read);
        assert_eq!((now.indent_size, now.indent_with_tabs, now.format_on_save, now.word_wrap), (2, false, true, true));
        assert_eq!(now.theme, user.theme, "only the editing settings");
        assert_eq!(now.indent_for("Python"), crate::file_style::Indent::Spaces(4));
        assert!(!now.format_on_save_for("TypeScript") && !now.format_on_save_for("TSX"));
        assert!(now.format_on_save_for("Rust"));
        assert!(!now.languages.contains_key("markdown"), "nothing Null has for it");
        // .null's own win, a language's settings added to its.
        let own = serde_json::from_str::<JsonMap>(r#"{ "indent_size": 3, "languages": { "Python": { "word_wrap": false } } }"#)
            .unwrap();
        let (now, _) = merged(&user, &layered(read, &own));
        assert_eq!(now.indent_size, 3);
        assert_eq!(now.indent_for("python"), crate::file_style::Indent::Spaces(4));
        assert!(!now.word_wrap_for("Python"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The project's file and VS Code's as `use_project` takes them (no window needed).
    fn project_at(root: &std::path::Path, user: &Settings) -> (Settings, ProjectFile) {
        let path = project_file(root);
        let read = read_project(&path);
        let unreadable = read.is_err();
        let own = read.unwrap_or_default();
        let values = layered(vscode_settings(root), &own);
        let (now, shadowed) = merged(user, &values);
        (now, ProjectFile { path: Some(path), values, shadowed, own, unreadable })
    }

    #[test]
    fn project_settings_keep_to_their_own() {
        let dir = crate::tools::test_dir("settings-project-own");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".vscode")).unwrap();
        std::fs::create_dir_all(dir.join(".null")).unwrap();
        let user_path = dir.join("user.json");
        let mut user = Settings { own_theme: Some("Solar".into()), ..Settings::default() };
        user.languages.insert("Python".into(), LanguageSettings { indent_size: Some(2), format_on_save: Some(true), ..Default::default() });
        user.save_to(&user_path);
        std::fs::write(
            vscode_file(&dir),
            r#"{ "editor.formatOnSave": true, "[python]": { "editor.formatOnSave": false },
                 "[javascript]": { "editor.tabSize": 4 }, "[javascriptreact]": { "editor.formatOnSave": true } }"#,
        )
        .unwrap();
        std::fs::write(project_file(&dir), "{\n  // ours\n  \"languages\": { \"Go\": { \"word_wrap\": true } }\n}\n").unwrap();
        let (now, project) = project_at(&dir, &user);
        // What the project names over yours, field by field; VS Code's two names for one.
        assert_eq!(now.indent_for("Python"), crate::file_style::Indent::Spaces(2), "your Python indent stays");
        assert!(!now.format_on_save_for("Python"));
        assert_eq!(now.indent_for("JavaScript"), crate::file_style::Indent::Spaces(4));
        assert!(now.format_on_save_for("JavaScript"));
        // An unrelated change: VS Code's settings aren't copied into .null.
        let changed = Settings { font_size: 16., ..now.clone() };
        changed.save_with(&user_path, Some(project));
        let ours = std::fs::read_to_string(project_file(&dir)).unwrap();
        assert!(ours.contains("// ours") && !ours.contains("ython") && !ours.contains("format_on_save"), "{ours}");
        let yours = Settings::load_from(&user_path).unwrap();
        assert_eq!(yours.languages["Python"], user.languages["Python"], "yours, as they were");
        assert_eq!(yours.font_size, 16.);
        // Your own theme cleared: written so, not kept from the file.
        let (now, project) = project_at(&dir, &yours);
        Settings { own_theme: None, ..now }.save_with(&user_path, Some(project));
        assert_eq!(Settings::load_from(&user_path).unwrap().own_theme, None);
        // A project file that can't be read is never written over.
        let broken = "{ \"indent_size\": 2 \"oops\": 1 }";
        std::fs::write(project_file(&dir), broken).unwrap();
        let (now, project) = project_at(&dir, &yours);
        assert!(project.unreadable);
        Settings { format_on_save: !now.format_on_save, ..now }.save_with(&user_path, Some(project));
        assert_eq!(std::fs::read_to_string(project_file(&dir)).unwrap(), broken);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn numbers_written_by_hand_stay_as_written() {
        let a: serde_json::Value = serde_json::from_str("13.3").unwrap();
        let b = serde_json::to_value(13.3f32).unwrap();
        assert!(same_json(&a, &b));
        assert!(!same_json(&serde_json::json!(13), &serde_json::json!(14)));
        // A project's text size is kept to what Null shows.
        let (now, _) = merged(&Settings::default(), &serde_json::from_str(r#"{ "font_size": 40 }"#).unwrap());
        assert_eq!(now.font_size, MAX_FONT_SIZE);
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
        let saved: serde_json::Value =
            serde_json::from_str(&crate::snippets::without_comments(&std::fs::read_to_string(&path).unwrap())).unwrap();
        assert_eq!(saved["from_the_future"], serde_json::json!([1, 2]), "unknown, kept");
        assert_eq!(saved["theme"], "neon", "couldn't be read, kept as written");
        assert_eq!(saved["word_wrap"], settings.word_wrap);
        // Written into the file as it was: its comment and order kept, only the change added.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("{\n  // mine\n  \"theme\": \"neon\",\n  \"font_size\": 15,"), "{text}");
        assert!(!text.contains("\"tab_size\"") && !text.contains("\"indent_size\""), "defaults not added: {text}");
        // A value that's a map, changed in place, indented as its line.
        settings.keys.insert("ctrl-cmd-l".into(), Some("editor::SelectAllOccurrences".into()));
        settings.save_to(&path);
        settings.font_size = 16.;
        settings.save_to(&path);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("  // mine\n") && text.contains("\"font_size\": 16"), "{text}");
        assert!(text.contains("\"keys\": {\n    \"ctrl-cmd-l\""), "{text}");
        assert_eq!(Settings::load_from(&path).unwrap().keys, settings.keys);
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
