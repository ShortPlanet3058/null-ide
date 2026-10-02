//! The palette: one field for everything, steered by the first character.
//!
//! - nothing: files, recent ones first
//! - `>` commands, grouped by category
//! - `:` go to a line
//! - `?` ask the AI

use crate::fuzzy;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use gpui::{
    Action, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle,
    KeyBinding, MouseMoveEvent, ScrollHandle, SharedString, StyledText, Subscription, Window, actions, div, prelude::*,
    px,
};
use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

actions!(palette, [SelectNext, SelectPrevious, Confirm, Dismiss]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Palette");
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, ctx),
        KeyBinding::new("ctrl-n", SelectNext, ctx),
        KeyBinding::new("up", SelectPrevious, ctx),
        KeyBinding::new("ctrl-p", SelectPrevious, ctx),
        KeyBinding::new("enter", Confirm, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
    ]);
}

const ROW_HEIGHT: f32 = 34.;
const HEADER_HEIGHT: f32 = 28.;
const LIST_HEIGHT: f32 = 380.;
const MAX_RESULTS: usize = 200;
/// Shown before the rest when nothing is typed yet.
const MAX_RECENT: usize = 5;
/// Large projects are walked up to this many files.
const MAX_FILES: usize = 50_000;
const APPEAR: Duration = Duration::from_millis(140);

/// Where a command is listed, in this order.
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
    const ALL: [Category; 9] = [
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

/// What the palette starts with.
pub struct PaletteOptions {
    pub commands: Vec<Command>,
    pub root: PathBuf,
    /// Files opened lately, most recent first (the current one left out).
    pub recent_files: Vec<PathBuf>,
    /// Names of the actions run lately from the palette, most recent first.
    pub recent_commands: Vec<&'static str>,
    /// Lines in the current file, for `:`.
    pub line_count: Option<usize>,
    /// What's typed in the field to begin with, like ">" for commands.
    pub query: String,
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
}

enum Row {
    Header(&'static str),
    Item {
        item: Item,
        highlights: Vec<usize>,
        /// Commands found by searching show their category beside them.
        show_category: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Files,
    Commands,
    Line,
    Ask,
}

impl Mode {
    const ALL: [Mode; 4] = [Mode::Files, Mode::Commands, Mode::Line, Mode::Ask];

    /// The mode a query is in, and the query without its prefix.
    pub fn of(query: &str) -> (Mode, &str) {
        match query.chars().next() {
            Some('>') => (Mode::Commands, &query[1..]),
            Some(':') => (Mode::Line, &query[1..]),
            Some('?') => (Mode::Ask, &query[1..]),
            _ => (Mode::Files, query),
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Mode::Files => "",
            Mode::Commands => ">",
            Mode::Line => ":",
            Mode::Ask => "?",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Mode::Files => "Files",
            Mode::Commands => "Commands",
            Mode::Line => "Go to line",
            Mode::Ask => "Ask AI",
        }
    }
}

pub enum PaletteEvent {
    Dismissed,
    OpenFile(PathBuf),
    Run(Box<dyn Action>),
    /// Typed after a `?`: a question for the AI.
    Ask(String),
    /// Typed after a `:`: a line number to jump to.
    GoToLine(usize),
}

pub struct Palette {
    input: Entity<TextInput>,
    commands: Vec<Command>,
    files: Vec<FileEntry>,
    recent_files: Vec<FileEntry>,
    recent_commands: Vec<&'static str>,
    line_count: Option<usize>,
    mode: Mode,
    rows: Vec<Row>,
    /// Indices into `rows` of the rows that can be picked.
    choices: Vec<usize>,
    selected: usize,
    scroll: ScrollHandle,
    opened_at: Instant,
    /// What's typed after the prefix.
    rest: String,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Palette {
    pub fn new(options: PaletteOptions, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Search files by name", cx));
        let subscription = cx.subscribe(&input, |this, _, TextInputEvent::Changed, cx| this.update_matches(cx));
        let root = options.root.clone();
        cx.spawn(async move |this, cx| {
            let files = cx.background_executor().spawn(async move { list_files(&root) }).await;
            this.update(cx, |this, cx| {
                this.files = files;
                this.update_matches(cx);
            })
            .ok();
        })
        .detach();
        let recent_files = options
            .recent_files
            .into_iter()
            .filter(|p| p.is_file())
            .take(MAX_RECENT)
            .map(|p| FileEntry::new(p, &options.root))
            .collect();
        let mut palette = Self {
            input,
            commands: options.commands,
            files: Vec::new(),
            recent_files,
            recent_commands: options.recent_commands,
            line_count: options.line_count,
            mode: Mode::Files,
            rows: Vec::new(),
            choices: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            opened_at: Instant::now(),
            rest: String::new(),
            _subscription: subscription,
        };
        palette.set_query(&options.query, cx);
        palette.update_matches(cx);
        palette
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_text(text, cx);
            input.select_range(text.len()..text.len(), cx);
        });
    }

    /// Switches mode, keeping what was typed after the old prefix.
    fn switch_to(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().to_string();
        let (_, rest) = Mode::of(&query);
        let rest = if mode == Mode::Line { String::new() } else { rest.to_string() };
        self.set_query(&format!("{}{rest}", mode.prefix()), cx);
        window.focus(&self.input.focus_handle(cx));
    }

    fn update_matches(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().to_string();
        let (mode, rest) = Mode::of(&query);
        self.mode = mode;
        self.rest = rest.trim().to_string();
        self.rows.clear();
        match mode {
            Mode::Files => self.match_files(rest.trim()),
            Mode::Commands => self.match_commands(rest.trim()),
            Mode::Line | Mode::Ask => {}
        }
        self.choices =
            self.rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Item { .. })).map(|(i, _)| i).collect();
        self.selected = 0;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    fn item(&mut self, item: Item, highlights: Vec<usize>, show_category: bool) {
        self.rows.push(Row::Item { item, highlights, show_category });
    }

    fn match_files(&mut self, query: &str) {
        let recent: HashSet<PathBuf> = self.recent_files.iter().map(|f| f.path.clone()).collect();
        if query.is_empty() {
            if !self.recent_files.is_empty() {
                self.rows.push(Row::Header("Recently opened"));
                for i in 0..self.recent_files.len() {
                    self.item(Item::RecentFile(i), Vec::new(), false);
                }
                self.rows.push(Row::Header("All files"));
            }
            let all: Vec<usize> = (0..self.files.len()).filter(|&i| !recent.contains(&self.files[i].path)).collect();
            for i in all.into_iter().take(MAX_RESULTS) {
                self.item(Item::File(i), Vec::new(), false);
            }
            return;
        }
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
            self.item(Item::File(i), highlights, false);
        }
        // Nothing by that name: maybe it was a command.
        if self.rows.is_empty() {
            let commands = self.command_matches(query);
            if !commands.is_empty() {
                self.rows.push(Row::Header("No files match · commands"));
                for (i, highlights) in commands.into_iter().take(8) {
                    self.item(Item::Command(i), highlights, true);
                }
            }
        }
    }

    /// Commands matching `query`, best first. The category counts too, so "ai use"
    /// finds "Use NVIDIA" under AI.
    fn command_matches(&self, query: &str) -> Vec<(usize, Vec<usize>)> {
        let mut found: Vec<(i32, usize, Vec<usize>)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let recent = self.recent_commands.iter().position(|&n| n == c.action.name());
                let boost = recent.map_or(0, |r| 12 - r.min(12) as i32);
                if let Some((score, highlights)) = fuzzy::score(&c.label, query) {
                    return Some((score + boost + 5, i, highlights));
                }
                let prefix = format!("{} ", c.category.label());
                let (score, highlights) = fuzzy::score(&format!("{prefix}{}", c.label), query)?;
                let highlights = highlights.into_iter().filter_map(|b| b.checked_sub(prefix.len())).collect();
                Some((score + boost, i, highlights))
            })
            .collect();
        found.sort_by_key(|(score, _, _)| std::cmp::Reverse(*score));
        found.into_iter().map(|(_, i, h)| (i, h)).collect()
    }

    fn match_commands(&mut self, query: &str) {
        if !query.is_empty() {
            for (i, highlights) in self.command_matches(query) {
                self.item(Item::Command(i), highlights, true);
            }
            return;
        }
        let recent: Vec<usize> = self
            .recent_commands
            .iter()
            .filter_map(|name| self.commands.iter().position(|c| c.action.name() == *name))
            .take(MAX_RECENT)
            .collect();
        if !recent.is_empty() {
            self.rows.push(Row::Header("Recently used"));
            for i in recent {
                self.item(Item::Command(i), Vec::new(), true);
            }
        }
        for category in Category::ALL {
            let members: Vec<usize> =
                (0..self.commands.len()).filter(|&i| self.commands[i].category == category).collect();
            if members.is_empty() {
                continue;
            }
            self.rows.push(Row::Header(category.label()));
            for i in members {
                self.item(Item::Command(i), Vec::new(), false);
            }
        }
    }

    fn select(&mut self, choice: usize, cx: &mut Context<Self>) {
        if choice < self.choices.len() && choice != self.selected {
            self.selected = choice;
            // Bring the group's header into view along with its first item.
            let row = self.choices[choice];
            let header_above = row > 0 && matches!(self.rows[row - 1], Row::Header(_));
            self.scroll.scroll_to_item(if header_above && choice != 0 { row - 1 } else { row });
            if choice == 0 {
                self.scroll.set_offset(gpui::point(px(0.), px(0.)));
            }
            cx.notify();
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.choices.is_empty() {
            self.select((self.selected + 1) % self.choices.len(), cx);
        }
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if !self.choices.is_empty() {
            self.select((self.selected + self.choices.len() - 1) % self.choices.len(), cx);
        }
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm_at(self.selected, cx);
    }

    fn line_target(&self) -> Option<usize> {
        self.rest.parse().ok().filter(|&n| n > 0)
    }

    fn confirm_at(&mut self, choice: usize, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Line => {
                if let Some(line) = self.line_target() {
                    cx.emit(PaletteEvent::GoToLine(line));
                }
                return;
            }
            Mode::Ask => {
                if !self.rest.is_empty()
                    && cx.global::<crate::settings::Settings>().ai.provider != crate::ai::ProviderId::Off
                {
                    cx.emit(PaletteEvent::Ask(self.rest.clone()));
                }
                return;
            }
            Mode::Files | Mode::Commands => {}
        }
        let item = self.choices.get(choice).and_then(|&row| match &self.rows[row] {
            Row::Item { item, .. } => Some(*item),
            Row::Header(_) => None,
        });
        match item {
            Some(Item::Command(i)) => cx.emit(PaletteEvent::Run(self.commands[i].action.boxed_clone())),
            Some(Item::File(i)) => cx.emit(PaletteEvent::OpenFile(self.files[i].path.clone())),
            Some(Item::RecentFile(i)) => cx.emit(PaletteEvent::OpenFile(self.recent_files[i].path.clone())),
            None => {}
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismissed);
    }

    fn render_header(&self, label: &'static str, first: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>();
        div()
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .flex()
            .items_end()
            .px(px(14.))
            .pb(px(6.))
            .when(!first, |d| d.mt(px(4.)))
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.faint)
            .child(label.to_uppercase())
            .into_any_element()
    }

    fn render_item(&self, row: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>();
        let Row::Item { item, highlights, show_category } = &self.rows[row] else { unreachable!() };
        let choice = self.choices.iter().position(|&r| r == row).unwrap_or(0);
        let selected = choice == self.selected;
        let highlight =
            HighlightStyle { color: Some(theme.caret), font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() };
        let highlights_in = |text: &str, offset: usize| -> Vec<(Range<usize>, HighlightStyle)> {
            highlights
                .iter()
                .filter_map(|&b| {
                    let b = b.checked_sub(offset)?;
                    let len = text.get(b..)?.chars().next()?.len_utf8();
                    Some((b..b + len, highlight))
                })
                .collect()
        };
        let dim = if selected { theme.muted } else { theme.faint };
        let (marker, label, details): (_, StyledText, Vec<gpui::AnyElement>) = match *item {
            Item::Command(i) => {
                let command = &self.commands[i];
                let label = command.label.to_string();
                let marked = highlights_in(&label, 0);
                let mut details = Vec::new();
                if *show_category {
                    details.push(div().text_color(dim).child(command.category.label()).into_any_element());
                }
                if let Some(keys) = &command.keys {
                    details.push(
                        div()
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(5.))
                            .bg(theme.hairline)
                            .text_color(theme.muted)
                            .child(keys.clone())
                            .into_any_element(),
                    );
                }
                (
                    div().text_size(px(13.)).text_color(if selected { theme.caret } else { theme.faint }).child("›"),
                    StyledText::new(label).with_highlights(marked),
                    details,
                )
            }
            Item::File(_) | Item::RecentFile(_) => {
                let file = match *item {
                    Item::File(i) => &self.files[i],
                    Item::RecentFile(i) => &self.recent_files[i],
                    Item::Command(_) => unreachable!(),
                };
                let name = file.relative[file.name_start..].to_string();
                let marked = highlights_in(&name, file.name_start);
                let folder = file.relative[..file.name_start].trim_end_matches(['/', '\\']).to_string();
                (
                    div().size(px(5.)).rounded(px(2.)).bg(if selected { theme.caret } else { theme.faint }),
                    StyledText::new(name).with_highlights(marked),
                    (!folder.is_empty())
                        .then(|| div().text_color(dim).child(folder).into_any_element())
                        .into_iter()
                        .collect(),
                )
            }
        };
        div()
            .id(row)
            .h(px(ROW_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .rounded(px(9.))
            .text_size(px(14.))
            .text_color(theme.foreground)
            .when(selected, |r| r.bg(theme.accent_soft))
            .child(div().w(px(12.)).flex().justify_center().child(marker))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(label))
            .child(div().flex_none().flex().items_center().gap(px(10.)).text_size(px(12.)).children(details))
            .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                if this.selected != choice {
                    this.selected = choice;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.confirm_at(choice, cx)))
            .into_any_element()
    }

    /// The single row shown in `:` and `?` modes.
    fn render_prompt_row(&self, marker: &'static str, text: String, active: bool, cx: &App) -> gpui::AnyElement {
        let theme = cx.global::<Theme>();
        div()
            .p(px(6.))
            .child(
                div()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .rounded(px(9.))
                    .bg(theme.accent_soft)
                    .text_size(px(14.))
                    .text_color(if active { theme.foreground } else { theme.faint })
                    .child(div().w(px(12.)).text_color(theme.caret).child(marker))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(text)),
            )
            .into_any_element()
    }

    /// The modes along the bottom: the current one lit, each one a click away.
    fn render_modes(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Theme>().clone();
        let current = self.mode;
        div().flex().items_center().gap(px(4.)).children(Mode::ALL.into_iter().map(|mode| {
            let active = mode == current;
            let prefix = div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.caret.opacity(if active { 1. } else { 0.7 }))
                .child(mode.prefix());
            div()
                .id(mode.label())
                .flex()
                .items_center()
                .gap(px(5.))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(6.))
                .cursor_pointer()
                .when(active, |d| d.bg(theme.accent_soft).text_color(theme.foreground))
                .when(!active, |d| d.hover(|d| d.text_color(theme.muted)))
                .when(!mode.prefix().is_empty(), |d| d.child(prefix))
                .child(mode.label())
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.switch_to(mode, window, cx)))
        }))
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
        let ai = cx.global::<crate::settings::Settings>().ai.provider;
        let list = match self.mode {
            Mode::Line => {
                let text = match (self.line_target(), self.line_count) {
                    (Some(n), Some(total)) if n > total => format!("Go to line {n} (the file has {total})"),
                    (Some(n), _) => format!("Go to line {n}"),
                    (None, Some(total)) => format!("Type a line number, 1 to {total}"),
                    (None, None) => "Type a line number".to_string(),
                };
                self.render_prompt_row(":", text, self.line_target().is_some(), cx)
            }
            Mode::Ask => {
                let (marker, text, active) = if ai == crate::ai::ProviderId::Off {
                    ("!", "AI is off. Choose a provider first: type “>use”.".to_string(), false)
                } else if self.rest.is_empty() {
                    ("?", format!("Ask {} about this file…", ai.label()), false)
                } else {
                    ("?", format!("Ask {}: {}", ai.label(), self.rest), true)
                };
                self.render_prompt_row(marker, text, active, cx)
            }
            Mode::Files | Mode::Commands if self.choices.is_empty() => {
                let theme = cx.global::<Theme>();
                let message = match self.mode {
                    Mode::Files if self.files.is_empty() && self.rest.is_empty() => "Looking for files…",
                    Mode::Files => "No matching files or commands",
                    _ => "No matching commands",
                };
                div()
                    .h(px(ROW_HEIGHT * 2.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(theme.faint)
                    .child(message)
                    .into_any_element()
            }
            Mode::Files | Mode::Commands => {
                let rows: Vec<gpui::AnyElement> = (0..self.rows.len())
                    .map(|i| match &self.rows[i] {
                        Row::Header(label) => self.render_header(label, i == 0, cx),
                        Row::Item { .. } => self.render_item(i, cx),
                    })
                    .collect();
                div()
                    .id("palette-results")
                    .max_h(px(LIST_HEIGHT))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .children(rows)
                    .into_any_element()
            }
        };
        let enter_hint = match self.mode {
            Mode::Files => "↵ open",
            Mode::Commands => "↵ run",
            Mode::Line => "↵ go",
            Mode::Ask => "↵ ask",
        };
        let modes = self.render_modes(cx);
        let theme = cx.global::<Theme>();
        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(theme.hairline)
            .text_size(px(12.))
            .text_color(theme.faint)
            .child(modes)
            .child(div().flex().gap(px(14.)).pr(px(8.)).child(enter_hint).child("esc close"));
        div()
            .key_context("Palette")
            // Clicks inside the palette shouldn't reach the backdrop, which closes it.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(640.))
            .max_w_full()
            .mt(px(-8. * (1. - eased)))
            .opacity(eased)
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(14.))
            .border_1()
            .border_color(theme.hairline)
            .bg(theme.raised)
            .shadow_lg()
            .child(
                div()
                    .px(px(20.))
                    .py(px(14.))
                    .border_b_1()
                    .border_color(theme.hairline)
                    .text_size(px(17.))
                    .line_height(px(26.))
                    .child(self.input.clone()),
            )
            .child(list)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};

    actions!(test, [One, Two, Three]);

    fn palette(cx: &mut TestAppContext, query: &str) -> Entity<Palette> {
        let commands = vec![
            Command { category: Category::Edit, label: "Undo".into(), action: Box::new(One), keys: None },
            Command { category: Category::File, label: "Save".into(), action: Box::new(Two), keys: None },
            Command { category: Category::Ai, label: "Use NVIDIA".into(), action: Box::new(Three), keys: None },
        ];
        let options = PaletteOptions {
            commands,
            root: PathBuf::from("/nonexistent"),
            recent_files: Vec::new(),
            recent_commands: vec![One.name()],
            line_count: Some(10),
            query: query.into(),
        };
        cx.new(|cx| Palette::new(options, cx))
    }

    fn rows(cx: &mut TestAppContext, palette: &Entity<Palette>) -> Vec<String> {
        palette.read_with(cx, |p, _| {
            p.rows
                .iter()
                .map(|r| match r {
                    Row::Header(h) => format!("# {h}"),
                    Row::Item { item: Item::Command(i), .. } => p.commands[*i].label.to_string(),
                    Row::Item { .. } => "file".into(),
                })
                .collect()
        })
    }

    #[gpui::test]
    fn commands_are_grouped_with_recent_ones_first(cx: &mut TestAppContext) {
        let p = palette(cx, ">");
        assert_eq!(rows(cx, &p), ["# Recently used", "Undo", "# File", "Save", "# Edit", "Undo", "# AI", "Use NVIDIA"]);
        // The category counts when searching: "ai use" finds the AI command.
        p.update(cx, |p, cx| p.set_query(">ai use", cx));
        assert_eq!(rows(cx, &p), ["Use NVIDIA"]);
    }

    #[gpui::test]
    fn a_file_search_with_no_files_falls_back_to_commands(cx: &mut TestAppContext) {
        let p = palette(cx, "save");
        assert_eq!(rows(cx, &p), ["# No files match · commands", "Save"]);
    }

    #[test]
    fn the_first_character_picks_the_mode() {
        assert_eq!(Mode::of("main.rs"), (Mode::Files, "main.rs"));
        assert_eq!(Mode::of(">save"), (Mode::Commands, "save"));
        assert_eq!(Mode::of(":42"), (Mode::Line, "42"));
        assert_eq!(Mode::of("?why"), (Mode::Ask, "why"));
        assert_eq!(Mode::of(""), (Mode::Files, ""));
    }
}
