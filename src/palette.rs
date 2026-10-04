//! The floating search box, in a few kinds that each do one thing:
//!
//! - Files (⌘P): go to a file, the recent ones first.
//! - Quick (⌘K): the settings changed all the time, changed right in the list
//!   (switches, ←→ on choices), and every command once you type.
//! - Line (⌃G): go to a line.

use crate::fuzzy;
use crate::settings::Settings;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::{Theme, ThemeName};
use crate::ui;
use gpui::{
    Action, AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    HighlightStyle, KeyBinding, KeyContext, MouseMoveEvent, ScrollHandle, SharedString, StyledText, Subscription,
    Window, actions, div, prelude::*, px, relative,
};
use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

actions!(palette, [SelectNext, SelectPrevious, Confirm, Dismiss, AdjustLeft, AdjustRight]);

/// Registered after the text field's keys: on a choice row, ←→ change the choice
/// instead of moving the caret (the query is empty there anyway).
pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Palette");
    let adjusting = Some("(Palette && adjusting) > TextInput");
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, ctx),
        KeyBinding::new("ctrl-n", SelectNext, ctx),
        KeyBinding::new("up", SelectPrevious, ctx),
        KeyBinding::new("ctrl-p", SelectPrevious, ctx),
        KeyBinding::new("enter", Confirm, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
        KeyBinding::new("left", AdjustLeft, adjusting),
        KeyBinding::new("right", AdjustRight, adjusting),
    ]);
}

const ROW_HEIGHT: f32 = ui::ROW_LG;
const LIST_HEIGHT: f32 = 380.;
const MAX_RESULTS: usize = 100;
/// ⌘P with nothing typed shows this many recent files.
const MAX_RECENT: usize = 6;
/// ...or, before anything was opened, this many of the project's files.
const MAX_STARTER_FILES: usize = 8;
/// Large projects are walked up to this many files.
const MAX_FILES: usize = 50_000;
const APPEAR: Duration = Duration::from_millis(140);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteKind {
    Files,
    Quick,
    Line,
    /// Places in the code: where a symbol is used, or the problems found.
    Locations,
    /// A task for the AI, typed in one line.
    Task,
    /// A commit message.
    Commit,
}

/// What a place in the list is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocationKind {
    Reference,
    Error,
    Warning,
    /// A definition, with the word that defines it (`fn`, `class`...).
    Symbol(&'static str),
    /// A file an AI task changed, with the lines added and removed.
    FileChange(crate::ai_task::ChangeKind, usize, usize),
}

/// A place in the code, with the text to show for it.
#[derive(Clone, Debug)]
pub struct Location {
    pub path: PathBuf,
    pub position: lsp_types::Position,
    /// The line's code for a reference, the message for a problem.
    pub text: String,
    pub kind: LocationKind,
}

impl PaletteKind {
    fn placeholder(self) -> &'static str {
        match self {
            PaletteKind::Files => "Go to file",
            PaletteKind::Quick => "Quick settings and commands",
            PaletteKind::Line => "Go to line",
            PaletteKind::Locations => "Filter",
            PaletteKind::Task => "Describe the task",
            PaletteKind::Commit => "Commit message",
        }
    }
}

/// Where a command is listed, shown beside it when searching.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    File,
    Edit,
    Lines,
    Cursors,
    Go,
    View,
    Appearance,
    Ai,
    App,
}

impl Category {
    pub const ALL: [Category; 9] = [
        Category::File,
        Category::Edit,
        Category::Lines,
        Category::Cursors,
        Category::Go,
        Category::View,
        Category::Appearance,
        Category::Ai,
        Category::App,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::File => "File",
            Category::Edit => "Edit",
            Category::Lines => "Lines",
            Category::Cursors => "Cursors",
            Category::Go => "Go",
            Category::View => "View",
            Category::Appearance => "Appearance",
            Category::Ai => "AI",
            Category::App => "Null",
        }
    }
}

pub struct Command {
    pub category: Category,
    pub label: SharedString,
    pub action: Box<dyn Action>,
    /// The shortcut, formatted for display.
    pub keys: Option<String>,
}

/// The settings in ⌘K, changed without leaving the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Quick {
    Theme,
    TextSize,
    Wrap,
    Sidebar,
    Terminal,
    Suggestions,
    Fade,
    Ai,
    AllSettings,
}

impl Quick {
    const ALL: [Quick; 9] = [
        Quick::Theme,
        Quick::TextSize,
        Quick::Wrap,
        Quick::Sidebar,
        Quick::Terminal,
        Quick::Suggestions,
        Quick::Fade,
        Quick::Ai,
        Quick::AllSettings,
    ];

    fn label(self) -> &'static str {
        match self {
            Quick::Theme => "Theme",
            Quick::TextSize => "Text size",
            Quick::Wrap => "Wrap lines",
            Quick::Sidebar => "Sidebar",
            Quick::Terminal => "Terminal",
            Quick::Suggestions => "Suggestions while typing",
            Quick::Fade => "Fade bars while typing",
            Quick::Ai => "AI",
            Quick::AllSettings => "All settings",
        }
    }

    /// What Enter (or a click) does; for choices, the next one.
    fn action(self, settings: &Settings) -> Box<dyn Action> {
        use crate::workspace::*;
        match self {
            Quick::Theme => theme_action(next_theme(settings.theme, 1)),
            Quick::TextSize => Box::new(IncreaseFontSize),
            Quick::Wrap => Box::new(crate::menus::ToggleWordWrap),
            Quick::Sidebar => Box::new(ToggleSidebar),
            Quick::Terminal => Box::new(ToggleTerminal),
            Quick::Suggestions => Box::new(ToggleAutocomplete),
            Quick::Fade => Box::new(crate::menus::ToggleFadeWhileTyping),
            Quick::Ai => Box::new(ToggleAi),
            Quick::AllSettings => Box::new(OpenSettings),
        }
    }

    fn is_choice(self) -> bool {
        matches!(self, Quick::Theme | Quick::TextSize)
    }
}

const THEMES: [ThemeName; 6] = ThemeName::ALL;

fn next_theme(current: ThemeName, step: isize) -> ThemeName {
    let i = THEMES.iter().position(|t| *t == current).unwrap_or(0) as isize;
    THEMES[(i + step).rem_euclid(THEMES.len() as isize) as usize]
}

fn theme_action(theme: ThemeName) -> Box<dyn Action> {
    crate::workspace::theme_action(theme)
}

/// Commands that a quick setting already covers, left out of ⌘K's search.
fn covered_by_quick(name: &str) -> bool {
    matches!(
        name.rsplit("::").next().unwrap_or(name),
        "ToggleWordWrap"
            | "ToggleSidebar"
            | "ToggleTerminal"
            | "ToggleAutocomplete"
            | "ToggleFadeWhileTyping"
            | "ToggleAi"
            | "UseNullTheme"
            | "UseAshTheme"
            | "UseMidnightTheme"
            | "UseMossTheme"
            | "UsePaperTheme"
            | "UseDuneTheme"
            | "IncreaseFontSize"
            | "DecreaseFontSize"
            | "OpenSettings"
    )
}

/// What the palette starts with.
pub struct PaletteOptions {
    pub kind: PaletteKind,
    pub commands: Vec<Command>,
    pub root: PathBuf,
    /// Files opened lately, most recent first (the current one left out).
    pub recent_files: Vec<PathBuf>,
    /// Names of the actions run lately, most recent first; they rank higher.
    pub recent_commands: Vec<&'static str>,
    /// Lines in the current file, for going to a line.
    pub line_count: Option<usize>,
    pub terminal_open: bool,
    /// For a list of places: what it is ("Problems", "References to x") and the places.
    pub title: Option<String>,
    pub locations: Vec<Location>,
}

struct FileEntry {
    path: PathBuf,
    relative: String,
    /// Byte offset where the file name starts in `relative`.
    name_start: usize,
}

impl FileEntry {
    fn new(path: PathBuf, root: &Path) -> Self {
        let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
        let name_start = relative.rfind(['/', '\\']).map_or(0, |i| i + 1);
        Self { path, relative, name_start }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Command(usize),
    File(usize),
    RecentFile(usize),
    Quick(Quick),
    /// ":42" typed in ⌘P.
    Line(usize),
    Location(usize),
}

struct Row {
    item: Item,
    highlights: Vec<usize>,
    /// Commands found by searching show their category beside them.
    show_category: bool,
}

pub enum PaletteEvent {
    Dismissed,
    OpenFile(PathBuf),
    /// Run a command and close.
    Run(Box<dyn Action>),
    /// Change a setting and stay open, so its effect shows at once.
    Apply(Box<dyn Action>),
    GoToLine(usize),
    OpenLocation(PathBuf, lsp_types::Position),
    /// The keyboard is on a place in the open file: show it, without going there yet.
    Preview(PathBuf, lsp_types::Position),
    /// Start an AI task with this description.
    StartTask(String),
    /// Commit every change with this message.
    Commit(String),
}

pub struct Palette {
    kind: PaletteKind,
    input: Entity<TextInput>,
    commands: Vec<Command>,
    files: Vec<FileEntry>,
    files_loaded: bool,
    recent_files: Vec<FileEntry>,
    recent_commands: Vec<&'static str>,
    line_count: Option<usize>,
    terminal_open: bool,
    title: Option<String>,
    locations: Vec<Location>,
    /// Symbols of the open file: their line is enough, and moving through them shows each.
    in_file: bool,
    root: PathBuf,
    rows: Vec<Row>,
    selected: usize,
    /// Where the pointer was when the keyboard last moved the selection.
    pointer_anchor: Option<gpui::Point<gpui::Pixels>>,
    scroll: ScrollHandle,
    opened_at: Instant,
    query: String,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Palette {
    pub fn new(options: PaletteOptions, cx: &mut Context<Self>) -> Self {
        let kind = options.kind;
        // One-line entries keep their own placeholder; their title is the line under them.
        let placeholder = match kind {
            PaletteKind::Task | PaletteKind::Commit => kind.placeholder().to_string(),
            _ => options.title.clone().unwrap_or_else(|| kind.placeholder().to_string()),
        };
        let input = cx.new(|cx| TextInput::new(placeholder, cx));
        let subscription = cx.subscribe(&input, |this, _, TextInputEvent::Changed, cx| this.update_rows(cx));
        if kind == PaletteKind::Files {
            let root = options.root.clone();
            cx.spawn(async move |this, cx| {
                let files = cx.background_executor().spawn(async move { list_files(&root) }).await;
                this.update(cx, |this, cx| {
                    this.files = files;
                    this.files_loaded = true;
                    this.update_rows(cx);
                })
                .ok();
            })
            .detach();
        }
        let recent_files = options
            .recent_files
            .into_iter()
            .filter(|p| p.is_file())
            .take(MAX_RECENT)
            .map(|p| FileEntry::new(p, &options.root))
            .collect();
        let mut palette = Self {
            kind,
            input,
            commands: options.commands,
            files: Vec::new(),
            files_loaded: false,
            recent_files,
            recent_commands: options.recent_commands,
            line_count: options.line_count,
            terminal_open: options.terminal_open,
            title: options.title,
            in_file: matches!(options.locations.first(), Some(l) if matches!(l.kind, LocationKind::Symbol(_)))
                && options.locations.iter().all(|l| Some(&l.path) == options.locations.first().map(|f| &f.path)),
            locations: options.locations,
            root: options.root.clone(),
            rows: Vec::new(),
            selected: 0,
            pointer_anchor: None,
            scroll: ScrollHandle::new(),
            opened_at: Instant::now(),
            query: String::new(),
            _subscription: subscription,
        };
        palette.update_rows(cx);
        palette
    }

    /// Types `query` into the field, as if the person had.
    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_text(query, cx));
    }

    pub fn kind(&self) -> PaletteKind {
        self.kind
    }

    /// The workspace says when a quick setting it applied changed something only it knows.
    pub fn set_terminal_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.terminal_open = open;
        cx.notify();
    }

    fn update_rows(&mut self, cx: &mut Context<Self>) {
        self.query = self.input.read(cx).text().trim().to_string();
        let query = self.query.clone();
        self.rows.clear();
        match self.kind {
            PaletteKind::Files => self.file_rows(&query),
            PaletteKind::Quick => self.quick_rows(&query),
            PaletteKind::Line | PaletteKind::Task | PaletteKind::Commit => {}
            PaletteKind::Locations => self.location_rows(&query),
        }
        self.selected = 0;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.preview(cx);
        cx.notify();
    }

    /// Shows the chosen symbol of the open file in the editor behind.
    fn preview(&self, cx: &mut Context<Self>) {
        if let (true, Some(Row { item: Item::Location(i), .. })) = (self.in_file, self.rows.get(self.selected)) {
            let location = &self.locations[*i];
            cx.emit(PaletteEvent::Preview(location.path.clone(), location.position));
        }
    }

    fn push(&mut self, item: Item, highlights: Vec<usize>, show_category: bool) {
        self.rows.push(Row { item, highlights, show_category });
    }

    /// Places matching what's typed, by their text or their file.
    fn location_rows(&mut self, query: &str) {
        if matches!(self.locations.first(), Some(l) if matches!(l.kind, LocationKind::Symbol(_))) {
            return self.symbol_rows(query);
        }
        // A task's changed files, then what to do with all of them.
        if matches!(self.locations.first(), Some(l) if matches!(l.kind, LocationKind::FileChange(..))) {
            for i in 0..self.locations.len() {
                if query.is_empty() || fuzzy::score(&self.locations[i].text, query).is_some() {
                    let highlights = fuzzy::score(&self.locations[i].text, query).map(|(_, h)| h).unwrap_or_default();
                    self.push(Item::Location(i), highlights, false);
                }
            }
            for i in 0..self.commands.len() {
                if query.is_empty() || fuzzy::score(&self.commands[i].label, query).is_some() {
                    self.push(Item::Command(i), Vec::new(), false);
                }
            }
            return;
        }
        for i in 0..self.locations.len() {
            let location = &self.locations[i];
            let file = location.path.strip_prefix(&self.root).unwrap_or(&location.path).display().to_string();
            let matched = query.is_empty() || fuzzy::score(&format!("{} {file}", location.text), query).is_some();
            if matched {
                let highlights = fuzzy::score(&location.text, query).map(|(_, h)| h).unwrap_or_default();
                self.push(Item::Location(i), highlights, false);
            }
        }
    }

    /// Symbols by name: in file order before typing, best match first after.
    fn symbol_rows(&mut self, query: &str) {
        const SHOWN: usize = 300;
        if query.is_empty() {
            // A whole project's worth is too many to scroll: it starts with what's typed.
            if self.in_file {
                for i in 0..self.locations.len().min(SHOWN) {
                    self.push(Item::Location(i), Vec::new(), false);
                }
            }
            return;
        }
        let mut found: Vec<(i32, usize, Vec<usize>)> = self
            .locations
            .iter()
            .enumerate()
            .filter_map(|(i, l)| fuzzy::score(&l.text, query).map(|(score, h)| (score, i, h)))
            .collect();
        found.sort_by_key(|(score, i, _)| (std::cmp::Reverse(*score), *i));
        for (_, i, highlights) in found.into_iter().take(SHOWN) {
            self.push(Item::Location(i), highlights, false);
        }
    }

    fn file_rows(&mut self, query: &str) {
        // A hidden extra for those who know it: ":42" goes to line 42.
        if let Some(line) = query.strip_prefix(':').and_then(|n| n.trim().parse().ok()) {
            return self.push(Item::Line(line), Vec::new(), false);
        }
        if query.is_empty() {
            if self.recent_files.is_empty() {
                for i in 0..self.files.len().min(MAX_STARTER_FILES) {
                    self.push(Item::File(i), Vec::new(), false);
                }
            } else {
                for i in 0..self.recent_files.len() {
                    self.push(Item::RecentFile(i), Vec::new(), false);
                }
            }
            return;
        }
        let recent: HashSet<PathBuf> = self.recent_files.iter().map(|f| f.path.clone()).collect();
        let mut found: Vec<(i32, usize, Vec<usize>)> = self
            .files
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                // A match within the file name beats one spread across folders.
                let in_name = fuzzy::score(&f.relative[f.name_start..], query)
                    .map(|(s, h)| (s + 15, h.into_iter().map(|b| b + f.name_start).collect()));
                let in_path = fuzzy::score(&f.relative, query);
                let (score, highlights) = match (in_name, in_path) {
                    (Some(a), Some(b)) => {
                        if a.0 >= b.0 {
                            a
                        } else {
                            b
                        }
                    }
                    (a, b) => a.or(b)?,
                };
                let boost = if recent.contains(&f.path) { 10 } else { 0 };
                Some((score + boost, i, highlights))
            })
            .collect();
        found.sort_by_key(|(score, _, _)| std::cmp::Reverse(*score));
        for (_, i, highlights) in found.into_iter().take(MAX_RESULTS) {
            self.push(Item::File(i), highlights, false);
        }
    }

    fn quick_rows(&mut self, query: &str) {
        if query.is_empty() {
            for quick in Quick::ALL {
                self.push(Item::Quick(quick), Vec::new(), false);
            }
            return;
        }
        // Quick settings and commands in one list, best match first.
        let mut found: Vec<(i32, Item, Vec<usize>)> = Quick::ALL
            .into_iter()
            .filter_map(|q| fuzzy::score(q.label(), query).map(|(s, h)| (s + 8, Item::Quick(q), h)))
            .collect();
        for (i, c) in self.commands.iter().enumerate() {
            if covered_by_quick(c.action.name()) {
                continue;
            }
            let recent = self.recent_commands.iter().position(|&n| n == c.action.name());
            let boost = recent.map_or(0, |r| 12 - r.min(12) as i32);
            if let Some((score, highlights)) = fuzzy::score(&c.label, query) {
                found.push((score + boost + 5, Item::Command(i), highlights));
                continue;
            }
            // The category counts too, so "lines dup" finds "Duplicate Line".
            let prefix = format!("{} ", c.category.label());
            if let Some((score, highlights)) = fuzzy::score(&format!("{prefix}{}", c.label), query) {
                let highlights = highlights.into_iter().filter_map(|b| b.checked_sub(prefix.len())).collect();
                found.push((score + boost, Item::Command(i), highlights));
            }
        }
        found.sort_by_key(|(score, _, _)| std::cmp::Reverse(*score));
        for (_, item, highlights) in found.into_iter().take(MAX_RESULTS) {
            self.push(item, highlights, true);
        }
    }

    fn selected_item(&self) -> Option<Item> {
        self.rows.get(self.selected).map(|r| r.item)
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.pointer_anchor = None;
        if ix < self.rows.len() && ix != self.selected {
            self.selected = ix;
            self.scroll.scroll_to_item(ix);
            self.preview(cx);
            cx.notify();
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.rows.is_empty() {
            self.select((self.selected + 1) % self.rows.len(), cx);
        }
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if !self.rows.is_empty() {
            self.select((self.selected + self.rows.len() - 1) % self.rows.len(), cx);
        }
    }

    fn adjust(&mut self, step: isize, cx: &mut Context<Self>) {
        let settings = cx.global::<Settings>();
        let action: Box<dyn Action> = match self.selected_item() {
            Some(Item::Quick(Quick::Theme)) => theme_action(next_theme(settings.theme, step)),
            Some(Item::Quick(Quick::TextSize)) if step > 0 => Box::new(crate::workspace::IncreaseFontSize),
            Some(Item::Quick(Quick::TextSize)) => Box::new(crate::workspace::DecreaseFontSize),
            _ => return,
        };
        cx.emit(PaletteEvent::Apply(action));
    }

    fn adjust_left(&mut self, _: &AdjustLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.adjust(-1, cx);
    }

    fn adjust_right(&mut self, _: &AdjustRight, _: &mut Window, cx: &mut Context<Self>) {
        self.adjust(1, cx);
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm_at(self.selected, cx);
    }

    fn line_target(&self) -> Option<usize> {
        self.query.parse().ok().filter(|&n| n > 0)
    }

    fn confirm_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        match self.kind {
            PaletteKind::Line => {
                if let Some(line) = self.line_target() {
                    cx.emit(PaletteEvent::GoToLine(line));
                }
                return;
            }
            PaletteKind::Task => {
                if !self.query.is_empty() {
                    cx.emit(PaletteEvent::StartTask(self.query.clone()));
                }
                return;
            }
            PaletteKind::Commit => {
                if !self.query.is_empty() {
                    cx.emit(PaletteEvent::Commit(self.query.clone()));
                }
                return;
            }
            PaletteKind::Files | PaletteKind::Quick | PaletteKind::Locations => {}
        }
        let Some(item) = self.rows.get(ix).map(|r| r.item) else { return };
        match item {
            Item::Command(i) => cx.emit(PaletteEvent::Run(self.commands[i].action.boxed_clone())),
            Item::File(i) => cx.emit(PaletteEvent::OpenFile(self.files[i].path.clone())),
            Item::RecentFile(i) => cx.emit(PaletteEvent::OpenFile(self.recent_files[i].path.clone())),
            Item::Line(line) => cx.emit(PaletteEvent::GoToLine(line)),
            Item::Location(i) => {
                let location = &self.locations[i];
                cx.emit(PaletteEvent::OpenLocation(location.path.clone(), location.position))
            }
            Item::Quick(Quick::AllSettings) => {
                cx.emit(PaletteEvent::Run(Quick::AllSettings.action(cx.global::<Settings>())))
            }
            Item::Quick(quick) => cx.emit(PaletteEvent::Apply(quick.action(cx.global::<Settings>()))),
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismissed);
    }

    /// The control at the right of a quick setting, showing its current value.
    fn quick_control(&self, quick: Quick, window: &Window, cx: &App) -> AnyElement {
        let theme = cx.global::<Theme>();
        let settings = cx.global::<Settings>();
        let keys = |action: &dyn Action| {
            window.highest_precedence_binding_for_action(action).map(|b| format_keys(&b)).map(|k| ui::key_cap(k, theme))
        };
        let switch = |on: bool| ui::switch(on, theme);
        let row = div().flex().items_center().gap(px(8.));
        match quick {
            // Six are too many to show at once: the current one, and ←→ for the others.
            Quick::Theme => row
                .text_size(px(ui::T_SM))
                .child(ui::key_cap("←", theme))
                .child(div().min_w(px(64.)).text_center().text_color(theme.foreground).child(settings.theme.label()))
                .child(ui::key_cap("→", theme))
                .into_any_element(),
            Quick::TextSize => row
                .text_size(px(ui::T_SM))
                .child(ui::key_cap("−", theme))
                .child(
                    div()
                        .w(px(26.))
                        .text_center()
                        .text_color(theme.foreground)
                        .child(format!("{}", settings.font_size)),
                )
                .child(ui::key_cap("+", theme))
                .into_any_element(),
            Quick::AllSettings => row.children(keys(&crate::workspace::OpenSettings)).into_any_element(),
            _ => {
                let on = match quick {
                    Quick::Wrap => settings.word_wrap,
                    Quick::Sidebar => settings.sidebar_visible,
                    Quick::Terminal => self.terminal_open,
                    Quick::Suggestions => settings.autocomplete,
                    Quick::Fade => settings.fade_bars_while_typing,
                    Quick::Ai => settings.ai.enabled,
                    _ => false,
                };
                row.children(keys(quick.action(settings).as_ref())).child(switch(on)).into_any_element()
            }
        }
    }

    fn render_row(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>();
        let row = &self.rows[ix];
        let selected = ix == self.selected;
        let highlight =
            HighlightStyle { color: Some(theme.caret), font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() };
        let highlights_in = |text: &str, offset: usize| -> Vec<(Range<usize>, HighlightStyle)> {
            row.highlights
                .iter()
                .filter_map(|&b| {
                    let b = b.checked_sub(offset)?;
                    let len = text.get(b..)?.chars().next()?.len_utf8();
                    Some((b..b + len, highlight))
                })
                .collect()
        };
        let dim = theme.muted;
        let accent = if selected { theme.caret } else { theme.faint };
        let file_marker = || div().size(px(5.)).rounded(px(2.)).bg(accent).into_any_element();
        let (marker, label, right): (AnyElement, AnyElement, Option<AnyElement>) = match row.item {
            Item::Command(i) => {
                let command = &self.commands[i];
                let label = command.label.to_string();
                let marked = highlights_in(&label, 0);
                let right = div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .when(row.show_category, |d| d.child(div().text_color(dim).child(command.category.label())))
                    .children(command.keys.clone().map(|k| ui::key_cap(k, theme)));
                (
                    div().text_size(px(13.)).text_color(accent).child("›").into_any_element(),
                    StyledText::new(label).with_highlights(marked).into_any_element(),
                    Some(right.into_any_element()),
                )
            }
            Item::Quick(quick) => {
                let label = quick.label().to_string();
                let marked = highlights_in(&label, 0);
                // Its control says what it is: no marker needed.
                (
                    div().into_any_element(),
                    StyledText::new(label).with_highlights(marked).into_any_element(),
                    Some(self.quick_control(quick, window, cx)),
                )
            }
            Item::File(_) | Item::RecentFile(_) => {
                let file = match row.item {
                    Item::File(i) => &self.files[i],
                    Item::RecentFile(i) => &self.recent_files[i],
                    _ => unreachable!(),
                };
                let name = file.relative[file.name_start..].to_string();
                let marked = highlights_in(&name, file.name_start);
                let folder = file.relative[..file.name_start].trim_end_matches(['/', '\\']).to_string();
                (
                    file_marker(),
                    StyledText::new(name).with_highlights(marked).into_any_element(),
                    (!folder.is_empty()).then(|| div().text_color(dim).child(folder).into_any_element()),
                )
            }
            Item::Line(line) => (
                div().text_size(px(13.)).text_color(accent).child(":").into_any_element(),
                div().child(format!("Go to line {line}")).into_any_element(),
                None,
            ),
            Item::Location(i) => {
                let location = &self.locations[i];
                let color = match location.kind {
                    LocationKind::Error => theme.error,
                    LocationKind::Warning => theme.warning,
                    LocationKind::Reference | LocationKind::Symbol(_) | LocationKind::FileChange(..) => accent,
                };
                if let LocationKind::FileChange(kind, added, removed) = location.kind {
                    use crate::ai_task::ChangeKind;
                    let text = StyledText::new(location.text.clone()).with_highlights(highlights_in(&location.text, 0));
                    let (dot, place) = match kind {
                        ChangeKind::Added => (theme.git_added, format!("new · +{added}")),
                        ChangeKind::Changed => (theme.git_modified, format!("+{added} −{removed}")),
                        ChangeKind::Deleted => (theme.git_deleted, "deleted · ↵ restores it".to_string()),
                    };
                    (
                        div().size(px(5.)).rounded_full().bg(dot).into_any_element(),
                        div().child(text).into_any_element(),
                        Some(div().text_color(dim).child(place).into_any_element()),
                    )
                } else if let LocationKind::Symbol(kind) = location.kind {
                    // The name in the code font, the word that defines it beside it, faint.
                    let text = StyledText::new(location.text.clone()).with_highlights(highlights_in(&location.text, 0));
                    let place = if self.in_file {
                        format!("{}", location.position.line + 1)
                    } else {
                        let file =
                            location.path.strip_prefix(&self.root).unwrap_or(&location.path).display().to_string();
                        format!("{file}:{}", location.position.line + 1)
                    };
                    (
                        div().size(px(5.)).rounded_full().bg(color).into_any_element(),
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(8.))
                            .child(
                                div()
                                    .font_family(cx.global::<crate::fonts::Fonts>().code.clone())
                                    .text_size(px(ui::T_MD))
                                    .child(text),
                            )
                            .child(div().text_size(px(ui::T_SM)).text_color(dim).child(kind))
                            .into_any_element(),
                        Some(div().text_color(dim).child(place).into_any_element()),
                    )
                } else {
                    let file = location.path.strip_prefix(&self.root).unwrap_or(&location.path).display().to_string();
                    let marked = highlights_in(&location.text, 0);
                    let text = StyledText::new(location.text.clone()).with_highlights(marked);
                    // Code shows in the code font; a problem's message in the interface one.
                    let label = if location.kind == LocationKind::Reference {
                        div()
                            .font_family(cx.global::<crate::fonts::Fonts>().code.clone())
                            .text_size(px(13.))
                            .child(text)
                    } else {
                        div().child(text)
                    };
                    (
                        div().size(px(5.)).rounded_full().bg(color).into_any_element(),
                        label.into_any_element(),
                        Some(
                            div()
                                .text_color(dim)
                                .child(format!("{file}:{}", location.position.line + 1))
                                .into_any_element(),
                        ),
                    )
                }
            }
        };
        let separated = row.item == Item::Quick(Quick::AllSettings);
        div()
            .id(ix)
            .flex_none()
            .when(separated, |d| d.mt(px(5.)).pt(px(5.)).border_t_1().border_color(theme.hairline))
            .child(
                div()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .rounded(px(ui::R_ROW_LG))
                    .text_size(px(ui::T_LG))
                    .cursor_pointer()
                    .text_color(if separated { theme.muted } else { theme.foreground })
                    .when(selected, |r| r.bg(theme.accent_soft))
                    .child(div().w(px(12.)).flex().justify_center().child(marker))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    // Paths and keys give way before the name does.
                    .children(right.map(|r| {
                        div().flex_shrink().min_w_0().max_w(relative(0.45)).truncate().text_size(px(ui::T_SM)).child(r)
                    })),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                // Only a real move takes over from the keyboard, not a pointer resting there
                // or a twitch of the trackpad.
                let anchor = *this.pointer_anchor.get_or_insert(event.position);
                let moved = (event.position.x - anchor.x).abs() + (event.position.y - anchor.y).abs();
                if moved > px(4.) && this.selected != ix {
                    this.selected = ix;
                    cx.notify();
                }
            }))
            .active(|s| s.opacity(0.7))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.confirm_at(ix, cx)))
            .into_any_element()
    }

    /// The one line shown when there's no list: going to a line, asking, or nothing found.
    fn render_message(&self, text: String, active: bool, cx: &App) -> AnyElement {
        let theme = cx.global::<Theme>();
        div()
            .p(px(6.))
            .child(
                div()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .px(px(14.))
                    .rounded(px(ui::R_ROW_LG))
                    .when(active, |d| d.bg(theme.accent_soft))
                    .text_size(px(ui::T_LG))
                    .text_color(if active { theme.foreground } else { theme.muted })
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(text)),
            )
            .into_any_element()
    }

    /// What Enter does right now, shown at the end of the field.
    fn hint(&self) -> &'static str {
        match self.kind {
            PaletteKind::Files => "↵ open",
            PaletteKind::Line | PaletteKind::Locations => "↵ go",
            PaletteKind::Task => "↵ start",
            PaletteKind::Commit => "↵ commit",
            PaletteKind::Quick => match self.selected_item() {
                Some(Item::Quick(q)) if q.is_choice() => "←→ change",
                Some(Item::Quick(Quick::AllSettings)) | Some(Item::Command(_)) => "↵ run",
                Some(Item::Quick(_)) => "↵ switch",
                _ => "",
            },
        }
    }
}

/// Every file in the project, respecting `.gitignore`, sorted by path.
fn list_files(root: &Path) -> Vec<FileEntry> {
    let mut files: Vec<FileEntry> = ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .take(MAX_FILES)
        .map(|e| FileEntry::new(e.into_path(), root))
        .collect();
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    files
}

/// The shortcut for `action` with the current keymap, formatted, whatever has focus
/// (asking the window only knows the keys of what's focused). The last binding added wins,
/// as a keymap preset's do over the defaults.
pub fn shortcut(action: &dyn Action, cx: &App) -> Option<String> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap.bindings_for_action(action).last().map(format_keys)
}

/// Formats a shortcut the way the platform shows them: ⌘⇧] on macOS, Ctrl+Shift+] elsewhere.
pub fn format_keys(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|stroke| {
            let m = stroke.modifiers();
            let key = match stroke.key() {
                "enter" => "↵".to_string(),
                "tab" => "⇥".to_string(),
                "escape" => "Esc".to_string(),
                "backspace" => "⌫".to_string(),
                "up" => "↑".to_string(),
                "down" => "↓".to_string(),
                "left" => "←".to_string(),
                "right" => "→".to_string(),
                key if key.chars().count() == 1 => key.to_uppercase(),
                key => key[..1].to_uppercase() + &key[1..],
            };
            if cfg!(target_os = "macos") {
                let mut s = String::new();
                for (on, symbol) in [(m.control, "⌃"), (m.alt, "⌥"), (m.shift, "⇧"), (m.platform, "⌘")] {
                    if on {
                        s.push_str(symbol);
                    }
                }
                s + &key
            } else {
                let mut parts: Vec<&str> = Vec::new();
                for (on, name) in [(m.control, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.platform, "Super")] {
                    if on {
                        parts.push(name);
                    }
                }
                parts.push(&key);
                parts.join("+")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Focusable for Palette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = (self.opened_at.elapsed().as_secs_f32() / APPEAR.as_secs_f32()).min(1.);
        if t < 1. {
            window.request_animation_frame();
        }
        let eased = 1. - (1. - t).powi(3);
        let list = match self.kind {
            PaletteKind::Line => {
                let text = match (self.line_target(), self.line_count) {
                    (Some(n), Some(total)) if n > total => format!("Line {n} (the file has {total})"),
                    (Some(n), _) => format!("Line {n}"),
                    (None, Some(total)) => format!("A line number, 1 to {total}"),
                    (None, None) => "A line number".to_string(),
                };
                Some(self.render_message(text, self.line_target().is_some(), cx))
            }
            PaletteKind::Commit => {
                let text = self.title.clone().unwrap_or_default();
                Some(self.render_message(text, false, cx))
            }
            PaletteKind::Task => {
                let text = match &self.title {
                    Some(who) => format!("{who} changes the files; you review every change before it stays"),
                    None => "You review every change before it stays".into(),
                };
                Some(self.render_message(text, false, cx))
            }
            _ if self.rows.is_empty() => {
                let text = match self.kind {
                    PaletteKind::Files if !self.files_loaded => "Looking for files…".to_string(),
                    PaletteKind::Files if self.query.is_empty() => "No files in this folder".to_string(),
                    PaletteKind::Files => format!("No file named “{}”", self.query),
                    PaletteKind::Locations if self.locations.is_empty() => {
                        format!("Nothing in {}", self.title.as_deref().unwrap_or("this list").to_lowercase())
                    }
                    PaletteKind::Locations if self.query.is_empty() => {
                        format!("Type a name: {} functions, types and constants", self.locations.len())
                    }
                    _ => format!("Nothing called “{}”", self.query),
                };
                Some(self.render_message(text, false, cx))
            }
            _ => {
                let rows: Vec<AnyElement> = (0..self.rows.len()).map(|i| self.render_row(i, window, cx)).collect();
                Some(
                    div()
                        .id("palette-results")
                        .max_h(px(LIST_HEIGHT))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .p(px(6.))
                        .flex()
                        .flex_col()
                        .children(rows)
                        .into_any_element(),
                )
            }
        };
        let hint = self.hint();
        let mut context = KeyContext::new_with_defaults();
        context.add("Palette");
        if matches!(self.selected_item(), Some(Item::Quick(q)) if q.is_choice()) {
            context.add("adjusting");
        }
        let theme = cx.global::<Theme>();
        div()
            .key_context(context)
            // Clicks inside the palette shouldn't reach the backdrop, which closes it.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::adjust_left))
            .on_action(cx.listener(Self::adjust_right))
            .w(px(600.))
            .max_w_full()
            .mt(px(-8. * (1. - eased)))
            .opacity(eased)
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(ui::R_MODAL))
            .border_1()
            .border_color(theme.hairline)
            .bg(theme.raised)
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(20.))
                    .py(px(14.))
                    .text_size(px(ui::T_XL))
                    .line_height(px(24.))
                    .child(div().flex_1().min_w_0().child(self.input.clone()))
                    .child(div().flex_none().text_size(px(ui::T_SM)).text_color(theme.muted).child(hint)),
            )
            .children(list.map(|l| div().border_t_1().border_color(theme.hairline).child(l)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    actions!(test, [One, Two]);

    fn palette(cx: &mut TestAppContext, kind: PaletteKind) -> Entity<Palette> {
        cx.update(|cx| cx.set_global(Settings::default()));
        let commands = vec![
            Command { category: Category::Lines, label: "Duplicate Line".into(), action: Box::new(One), keys: None },
            Command { category: Category::File, label: "Save".into(), action: Box::new(Two), keys: None },
            Command {
                category: Category::View,
                label: "Wrap Lines".into(),
                action: Box::new(crate::menus::ToggleWordWrap),
                keys: None,
            },
        ];
        let options = PaletteOptions {
            kind,
            commands,
            root: PathBuf::from("/nonexistent"),
            recent_files: Vec::new(),
            recent_commands: Vec::new(),
            line_count: Some(10),
            terminal_open: false,
            title: None,
            locations: Vec::new(),
        };
        cx.new(|cx| Palette::new(options, cx))
    }

    fn type_query(cx: &mut TestAppContext, p: &Entity<Palette>, text: &str) {
        let input = p.read_with(cx, |p, _| p.input.clone());
        input.update(cx, |input, cx| input.set_text(text, cx));
    }

    fn items(cx: &mut TestAppContext, p: &Entity<Palette>) -> Vec<String> {
        p.read_with(cx, |p, _| {
            p.rows
                .iter()
                .map(|r| match r.item {
                    Item::Command(i) => p.commands[i].label.to_string(),
                    Item::Quick(q) => format!("quick: {}", q.label()),
                    Item::Line(n) => format!("line {n}"),
                    _ => "file".into(),
                })
                .collect()
        })
    }

    #[gpui::test]
    fn quick_settings_first_then_commands_when_typing(cx: &mut TestAppContext) {
        let p = palette(cx, PaletteKind::Quick);
        assert_eq!(items(cx, &p).len(), Quick::ALL.len());
        type_query(cx, &p, "dup");
        assert_eq!(items(cx, &p), ["Duplicate Line"]);
        // A command a quick setting covers isn't listed twice.
        type_query(cx, &p, "wrap");
        assert_eq!(items(cx, &p), ["quick: Wrap lines"]);
    }

    #[gpui::test]
    fn colon_and_a_number_in_files_goes_to_a_line(cx: &mut TestAppContext) {
        let p = palette(cx, PaletteKind::Files);
        type_query(cx, &p, ":42");
        assert_eq!(items(cx, &p), ["line 42"]);
    }

    #[gpui::test]
    fn a_list_of_places_filters_by_text_and_file(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(Settings::default()));
        let place = |file: &str, text: &str| Location {
            path: PathBuf::from(format!("/p/{file}")),
            position: lsp_types::Position::new(3, 0),
            text: text.into(),
            kind: LocationKind::Reference,
        };
        let options = PaletteOptions {
            kind: PaletteKind::Locations,
            commands: Vec::new(),
            root: PathBuf::from("/p"),
            recent_files: Vec::new(),
            recent_commands: Vec::new(),
            line_count: None,
            terminal_open: false,
            title: Some("2 uses of total".into()),
            locations: vec![place("a.rs", "let total = 1;"), place("b.rs", "print(total)")],
        };
        let p = cx.new(|cx| Palette::new(options, cx));
        assert_eq!(p.read_with(cx, |p, _| p.rows.len()), 2);
        type_query(cx, &p, "b.rs");
        assert_eq!(p.read_with(cx, |p, _| p.rows.len()), 1);
    }

    #[test]
    fn themes_cycle_both_ways() {
        assert_eq!(next_theme(ThemeName::Null, 1), ThemeName::Ash);
        assert_eq!(next_theme(ThemeName::Null, -1), ThemeName::Dune);
    }
}
