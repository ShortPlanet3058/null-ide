use crate::fuzzy;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use gpui::{
    Action, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle,
    KeyBinding, MouseMoveEvent, ScrollStrategy, SharedString, StyledText, Subscription, UniformListScrollHandle,
    Window, actions, div, prelude::*, px, uniform_list,
};
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
const VISIBLE_ROWS: usize = 10;
const MAX_RESULTS: usize = 200;
/// Large projects are walked up to this many files.
const MAX_FILES: usize = 50_000;
const APPEAR: Duration = Duration::from_millis(140);

pub struct Command {
    pub label: SharedString,
    pub action: Box<dyn Action>,
    /// The shortcut, formatted for display.
    pub keys: Option<String>,
}

struct FileEntry {
    path: PathBuf,
    relative: String,
    /// Byte offset where the file name starts in `relative`.
    name_start: usize,
}

#[derive(Clone, Copy)]
enum Item {
    Command(usize),
    File(usize),
}

struct Match {
    item: Item,
    score: i32,
    highlights: Vec<usize>,
}

pub enum PaletteEvent {
    Dismissed,
    OpenFile(PathBuf),
    Run(Box<dyn Action>),
    /// Typed after a `?`: a question for the AI.
    Ask(String),
}

/// Search across the project's files and every command, from one field.
pub struct Palette {
    input: Entity<TextInput>,
    commands: Vec<Command>,
    files: Vec<FileEntry>,
    matches: Vec<Match>,
    selected: usize,
    scroll: UniformListScrollHandle,
    opened_at: Instant,
    /// Set while the query starts with `?`.
    question: Option<String>,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Palette {
    pub fn new(commands: Vec<Command>, root: PathBuf, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Search files and commands, or type ? to ask AI", cx));
        let subscription = cx.subscribe(&input, |this, _, TextInputEvent::Changed, cx| this.update_matches(cx));
        cx.spawn(async move |this, cx| {
            let files = cx.background_executor().spawn(async move { list_files(&root) }).await;
            this.update(cx, |this, cx| {
                this.files = files;
                this.update_matches(cx);
            })
            .ok();
        })
        .detach();
        let mut palette = Self {
            input,
            commands,
            files: Vec::new(),
            matches: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            opened_at: Instant::now(),
            question: None,
            _subscription: subscription,
        };
        palette.update_matches(cx);
        palette
    }

    fn update_matches(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().to_string();
        // A leading `?` turns the palette into a question box.
        if let Some(question) = query.strip_prefix('?') {
            self.question = Some(question.trim().to_string());
            self.matches.clear();
            cx.notify();
            return;
        }
        self.question = None;
        let mut matches: Vec<Match> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                fuzzy::score(&c.label, &query).map(|(score, highlights)| Match {
                    item: Item::Command(i),
                    score,
                    highlights,
                })
            })
            .collect();
        if !query.trim().is_empty() {
            matches.extend(self.files.iter().enumerate().filter_map(|(i, f)| {
                // A match within the file name beats one spread across folders.
                let in_name = fuzzy::score(&f.relative[f.name_start..], &query)
                    .map(|(s, h)| (s + 15, h.into_iter().map(|b| b + f.name_start).collect()));
                let in_path = fuzzy::score(&f.relative, &query);
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
                Some(Match { item: Item::File(i), score, highlights })
            }));
            matches.sort_by_key(|m| std::cmp::Reverse(m.score));
        } else {
            matches.extend(self.files.iter().enumerate().take(MAX_RESULTS).map(|(i, _)| Match {
                item: Item::File(i),
                score: 0,
                highlights: Vec::new(),
            }));
        }
        matches.truncate(MAX_RESULTS);
        self.matches = matches;
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.matches.len() && ix != self.selected {
            self.selected = ix;
            self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
            cx.notify();
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.select((self.selected + 1) % self.matches.len(), cx);
        }
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if !self.matches.is_empty() {
            self.select((self.selected + self.matches.len() - 1) % self.matches.len(), cx);
        }
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm_at(self.selected, cx);
    }

    fn confirm_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(question) = &self.question {
            if !question.is_empty()
                && cx.global::<crate::settings::Settings>().ai.provider != crate::ai::ProviderId::Off
            {
                cx.emit(PaletteEvent::Ask(question.clone()));
            }
            return;
        }
        match self.matches.get(ix).map(|m| m.item) {
            Some(Item::Command(i)) => cx.emit(PaletteEvent::Run(self.commands[i].action.boxed_clone())),
            Some(Item::File(i)) => cx.emit(PaletteEvent::OpenFile(self.files[i].path.clone())),
            None => {}
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismissed);
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Theme>();
        let m = &self.matches[ix];
        let selected = ix == self.selected;
        let highlight =
            HighlightStyle { color: Some(theme.caret), font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() };
        let highlights_in = |text: &str, offset: usize| -> Vec<(Range<usize>, HighlightStyle)> {
            m.highlights
                .iter()
                .filter_map(|&b| {
                    let b = b.checked_sub(offset)?;
                    let len = text.get(b..)?.chars().next()?.len_utf8();
                    Some((b..b + len, highlight))
                })
                .collect()
        };
        let (marker, label, detail): (_, StyledText, Option<String>) = match m.item {
            Item::Command(i) => {
                let command = &self.commands[i];
                let label = command.label.to_string();
                let marked = highlights_in(&label, 0);
                (
                    div().text_size(px(13.)).text_color(if selected { theme.caret } else { theme.faint }).child("›"),
                    StyledText::new(label).with_highlights(marked),
                    command.keys.clone(),
                )
            }
            Item::File(i) => {
                let file = &self.files[i];
                let name = file.relative[file.name_start..].to_string();
                let marked = highlights_in(&name, file.name_start);
                let folder = file.relative[..file.name_start].trim_end_matches(['/', '\\']).to_string();
                (
                    div().size(px(5.)).rounded(px(2.)).bg(if selected { theme.caret } else { theme.faint }),
                    StyledText::new(name).with_highlights(marked),
                    (!folder.is_empty()).then_some(folder),
                )
            }
        };
        div()
            .id(ix)
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .rounded(px(9.))
            .text_size(px(14.))
            .text_color(theme.foreground)
            .when(selected, |row| row.bg(theme.accent_soft))
            .child(div().w(px(12.)).flex().justify_center().child(marker))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(label))
            .children(detail.map(|d| div().flex_none().text_size(px(12.)).text_color(theme.faint).child(d)))
            .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| this.select(ix, cx)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.confirm_at(ix, cx)))
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
        .filter_map(|e| {
            let relative = e.path().strip_prefix(root).ok()?.to_string_lossy().into_owned();
            let name_start = relative.rfind(['/', '\\']).map_or(0, |i| i + 1);
            Some(FileEntry { path: e.into_path(), relative, name_start })
        })
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
        let rows = self.matches.len();
        let list_height = (rows.clamp(1, VISIBLE_ROWS) as f32) * ROW_HEIGHT + 12.;
        let theme = cx.global::<Theme>();
        let footer = div()
            .flex()
            .gap(px(16.))
            .px(px(20.))
            .py(px(10.))
            .border_t_1()
            .border_color(theme.hairline)
            .text_size(px(12.))
            .text_color(theme.faint)
            .when(self.question.is_none(), |f| f.child("↑↓ to move").child("↵ to open"))
            .when(self.question.is_some(), |f| f.child("↵ to ask"))
            .child("Esc to close");
        let ai = cx.global::<crate::settings::Settings>().ai.provider;
        let list = if let Some(question) = &self.question {
            let (marker, text, color) = if ai == crate::ai::ProviderId::Off {
                ("!", "AI is off. Choose a provider first: type “AI: Use”.".to_string(), theme.muted)
            } else if question.is_empty() {
                ("?", format!("Ask {} about this file…", ai.label()), theme.faint)
            } else {
                ("?", format!("Ask {}: {question}", ai.label()), theme.foreground)
            };
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
                        .text_color(color)
                        .child(div().w(px(12.)).text_color(theme.caret).child(marker))
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(text)),
                )
                .into_any_element()
        } else if rows == 0 {
            div()
                .h(px(list_height))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(theme.faint)
                .child("No matching files or commands")
                .into_any_element()
        } else {
            uniform_list(
                "palette-results",
                rows,
                cx.processor(|this, range: Range<usize>, _, cx| range.map(|ix| this.render_row(ix, cx)).collect()),
            )
            .track_scroll(self.scroll.clone())
            .h(px(list_height))
            .p(px(6.))
            .into_any_element()
        };
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
