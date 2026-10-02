mod assist;
mod changes;
mod commands;
mod completion;
mod intel;

pub use completion::CompletionMenu;
pub use intel::HoverCard;

use crate::buffer::Buffer;
use crate::element::EditorElement;
use crate::find_bar::{CloseFind, DeployFind, DeployReplace, FindBar, FindNext, FindPrevious};
use crate::fonts::Fonts;
use crate::highlight::{Highlighter, Span};
use crate::languages;
use crate::lsp_store::LspStore;
use crate::search::SearchQuery;
use crate::settings::Settings;
use crate::theme::Theme;
use gpui::{
    AnyElement, App, Bounds, ClickEvent, ClipboardItem, Context, CursorStyle, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, HighlightStyle, KeyBinding, KeyContext, KeyDownEvent, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, ShapedLine, StyledText,
    Subscription, Task, UTF16Selection, Window, actions, anchored, deferred, div, point, prelude::*, px, size,
};
use regex::Regex;
use ropey::Rope;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

actions!(
    editor,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        MoveWordLeft,
        MoveWordRight,
        SelectWordLeft,
        SelectWordRight,
        MoveLineStart,
        MoveLineEnd,
        SelectLineStart,
        SelectLineEnd,
        MoveDocStart,
        MoveDocEnd,
        SelectDocStart,
        SelectDocEnd,
        PageUp,
        PageDown,
        Backspace,
        BackspaceWord,
        BackspaceLine,
        Delete,
        DeleteWord,
        Newline,
        Tab,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Save,
        GoToDefinition,
        ShowInfo,
        ShowCompletions,
        CompletionNext,
        CompletionPrevious,
        ConfirmCompletion,
        CancelCompletion,
        InlineAssist,
        ToggleComment,
        Indent,
        Outdent,
        MoveLineUp,
        MoveLineDown,
        DuplicateLineUp,
        DuplicateLineDown,
        DeleteLine,
        SelectLine,
    ]
);

pub const TAB_SIZE: usize = 4;
/// Edits of the same kind closer together than this undo as one step.
const UNDO_GROUP: Duration = Duration::from_millis(1000);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Editor");
    let mut keys = vec![
        KeyBinding::new("left", MoveLeft, ctx),
        KeyBinding::new("right", MoveRight, ctx),
        KeyBinding::new("up", MoveUp, ctx),
        KeyBinding::new("down", MoveDown, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new("home", MoveLineStart, ctx),
        KeyBinding::new("end", MoveLineEnd, ctx),
        KeyBinding::new("shift-home", SelectLineStart, ctx),
        KeyBinding::new("shift-end", SelectLineEnd, ctx),
        KeyBinding::new("pageup", PageUp, ctx),
        KeyBinding::new("pagedown", PageDown, ctx),
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("shift-backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("enter", Newline, ctx),
        KeyBinding::new("shift-enter", Newline, ctx),
        KeyBinding::new("tab", Tab, ctx),
        KeyBinding::new("secondary-a", SelectAll, ctx),
        KeyBinding::new("secondary-c", Copy, ctx),
        KeyBinding::new("secondary-x", Cut, ctx),
        KeyBinding::new("secondary-v", Paste, ctx),
        KeyBinding::new("secondary-z", Undo, ctx),
        KeyBinding::new("secondary-shift-z", Redo, ctx),
        KeyBinding::new("secondary-s", Save, ctx),
        KeyBinding::new("f12", GoToDefinition, ctx),
        KeyBinding::new("secondary-shift-i", ShowInfo, ctx),
        KeyBinding::new("ctrl-space", ShowCompletions, ctx),
        KeyBinding::new("secondary-i", InlineAssist, ctx),
        KeyBinding::new("secondary-/", ToggleComment, ctx),
        KeyBinding::new("secondary-]", Indent, ctx),
        KeyBinding::new("secondary-[", Outdent, ctx),
        KeyBinding::new("shift-tab", Outdent, ctx),
        KeyBinding::new("alt-up", MoveLineUp, ctx),
        KeyBinding::new("alt-down", MoveLineDown, ctx),
        KeyBinding::new("alt-shift-up", DuplicateLineUp, ctx),
        KeyBinding::new("alt-shift-down", DuplicateLineDown, ctx),
        KeyBinding::new("secondary-shift-k", DeleteLine, ctx),
        KeyBinding::new("secondary-l", SelectLine, ctx),
    ];
    if cfg!(target_os = "macos") {
        keys.extend([
            KeyBinding::new("alt-left", MoveWordLeft, ctx),
            KeyBinding::new("alt-right", MoveWordRight, ctx),
            KeyBinding::new("alt-shift-left", SelectWordLeft, ctx),
            KeyBinding::new("alt-shift-right", SelectWordRight, ctx),
            KeyBinding::new("cmd-left", MoveLineStart, ctx),
            KeyBinding::new("cmd-right", MoveLineEnd, ctx),
            KeyBinding::new("cmd-shift-left", SelectLineStart, ctx),
            KeyBinding::new("cmd-shift-right", SelectLineEnd, ctx),
            KeyBinding::new("cmd-up", MoveDocStart, ctx),
            KeyBinding::new("cmd-down", MoveDocEnd, ctx),
            KeyBinding::new("cmd-shift-up", SelectDocStart, ctx),
            KeyBinding::new("cmd-shift-down", SelectDocEnd, ctx),
            KeyBinding::new("alt-backspace", BackspaceWord, ctx),
            KeyBinding::new("cmd-backspace", BackspaceLine, ctx),
            KeyBinding::new("alt-delete", DeleteWord, ctx),
        ]);
    } else {
        keys.extend([
            KeyBinding::new("ctrl-left", MoveWordLeft, ctx),
            KeyBinding::new("ctrl-right", MoveWordRight, ctx),
            KeyBinding::new("ctrl-shift-left", SelectWordLeft, ctx),
            KeyBinding::new("ctrl-shift-right", SelectWordRight, ctx),
            KeyBinding::new("ctrl-home", MoveDocStart, ctx),
            KeyBinding::new("ctrl-end", MoveDocEnd, ctx),
            KeyBinding::new("ctrl-shift-home", SelectDocStart, ctx),
            KeyBinding::new("ctrl-shift-end", SelectDocEnd, ctx),
            KeyBinding::new("ctrl-backspace", BackspaceWord, ctx),
            KeyBinding::new("ctrl-delete", DeleteWord, ctx),
            KeyBinding::new("ctrl-y", Redo, ctx),
        ]);
    }
    // While the suggestion list is open these win over the editor's own keys.
    let menu = Some("Editor && showing_completions");
    keys.extend([
        KeyBinding::new("up", CompletionPrevious, menu),
        KeyBinding::new("down", CompletionNext, menu),
        KeyBinding::new("ctrl-p", CompletionPrevious, menu),
        KeyBinding::new("ctrl-n", CompletionNext, menu),
        KeyBinding::new("enter", ConfirmCompletion, menu),
        KeyBinding::new("tab", ConfirmCompletion, menu),
        KeyBinding::new("escape", CancelCompletion, menu),
    ]);
    cx.bind_keys(keys);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    fn caret(offset: usize) -> Self {
        Self { anchor: offset, head: offset }
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    Other,
}

struct Snapshot {
    text: Rope,
    selection: Selection,
}

pub enum EditorEvent {
    Edited,
    Saved,
    /// Save was asked for, but the buffer has no file yet.
    NeedsPath,
    /// The file changed on disk while there were unsaved edits here.
    ChangedOnDisk,
    /// Writing the file failed; the message says why.
    SaveFailed(String),
    /// Go to definition landed in another file.
    GoTo {
        path: PathBuf,
        range: lsp_types::Range,
    },
}

impl EventEmitter<EditorEvent> for Editor {}

/// Where things were drawn last frame, for mouse hit testing and IME.
pub struct Layout {
    /// Screen position of line 0, column 0, with scrolling applied.
    pub text_origin: Point<Pixels>,
    pub text_bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub char_width: Pixels,
    pub visible_lines: Range<usize>,
    pub shaped: Vec<ShapedLine>,
    /// None when everything fits and there's nothing to scroll.
    pub scrollbar: Option<ScrollbarLayout>,
}

pub struct ScrollbarLayout {
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
    /// How far the view can scroll, in pixels.
    pub max_scroll: f32,
}

pub struct Scroll {
    pub y: f32,
    pub target_y: f32,
    pub x: f32,
    pub last_tick: Option<Instant>,
}

pub struct CaretMotion {
    pub from: Point<Pixels>,
    pub to: Point<Pixels>,
    pub started: Instant,
    pub visual: Point<Pixels>,
    pub placed: bool,
}

/// The active search in an editor. Match ranges are char offsets, kept up to date as the text changes.
pub struct SearchState {
    pub query: SearchQuery,
    regex: Option<Regex>,
    pub matches: Vec<Range<usize>>,
    pub current: Option<usize>,
}

impl SearchState {
    pub fn is_invalid(&self) -> bool {
        self.regex.is_none() && !self.query.text.is_empty()
    }
}

pub struct Editor {
    focus_handle: FocusHandle,
    pub buffer: Buffer,
    path: Option<PathBuf>,
    highlighter: Option<Highlighter>,
    pub spans: Vec<Span>,
    pub selection: Selection,
    goal_column: Option<usize>,
    pub marked: Option<Range<usize>>,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    last_edit: Option<(EditKind, Instant)>,
    pub scroll: Scroll,
    pub caret: CaretMotion,
    /// Last caret move or edit. The caret stays solid until it has been idle a moment.
    pub last_activity: Instant,
    wake_task: Option<Task<()>>,
    pub layout: Option<Layout>,
    pub autoscroll: bool,
    dragging: Option<DragUnit>,
    pub font_size: Pixels,
    pub search: Option<SearchState>,
    find_bar: Option<Entity<FindBar>>,
    /// Reused by Find Next when the find bar is closed, and to prefill it.
    last_query: SearchQuery,
    lsp: Option<Entity<LspStore>>,
    lsp_version: i32,
    lsp_subscription: Option<Subscription>,
    pub hover: Option<HoverCard>,
    hover_word: Option<Range<usize>>,
    hover_task: Option<Task<()>>,
    hover_close_task: Option<Task<()>>,
    mouse_in_card: bool,
    /// Opened from the keyboard (or as a notice): the mouse doesn't close it.
    hover_from_keyboard: bool,
    /// Typing with Alt held (e.g. Alt+arrows) hides the card until Alt is released.
    hover_suppressed: bool,
    alt_held: bool,
    /// Cmd on macOS, Ctrl elsewhere: held to make words clickable for go to definition.
    secondary_held: bool,
    /// The word underlined as a link while Cmd/Ctrl is held.
    pub link_word: Option<Range<usize>>,
    mouse_position: Option<Point<Pixels>>,
    definition_task: Option<Task<()>>,
    /// While dragging the scrollbar: where on the thumb it was grabbed.
    scrollbar_drag: Option<Pixels>,
    pub completion: Option<CompletionMenu>,
    completion_task: Option<Task<()>>,
    /// The file as last committed, to mark changed lines in the gutter.
    git_base: Option<std::sync::Arc<str>>,
    pub git_hunks: Vec<crate::git::Hunk>,
    git_base_task: Option<Task<()>>,
    git_diff_task: Option<Task<()>>,
    /// The Cmd+I card, while it's open.
    assist: Option<(Entity<crate::inline_assist::InlineAssist>, Subscription)>,
}

impl Editor {
    pub fn new(buffer: Buffer, path: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let highlighter = path.as_deref().and_then(languages::for_path).and_then(Highlighter::new);
        let mut editor = Self {
            focus_handle: cx.focus_handle(),
            buffer,
            path,
            highlighter,
            spans: Vec::new(),
            selection: Selection::caret(0),
            goal_column: None,
            marked: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit: None,
            scroll: Scroll { y: 0., target_y: 0., x: 0., last_tick: None },
            caret: CaretMotion {
                from: Point::default(),
                to: Point::default(),
                started: Instant::now(),
                visual: Point::default(),
                placed: false,
            },
            last_activity: Instant::now(),
            wake_task: None,
            layout: None,
            autoscroll: false,
            dragging: None,
            font_size: px(cx.global::<Settings>().font_size),
            search: None,
            find_bar: None,
            last_query: SearchQuery::default(),
            lsp: None,
            lsp_version: 0,
            lsp_subscription: None,
            hover: None,
            hover_word: None,
            hover_task: None,
            hover_close_task: None,
            mouse_in_card: false,
            hover_from_keyboard: false,
            hover_suppressed: false,
            alt_held: false,
            secondary_held: false,
            link_word: None,
            mouse_position: None,
            definition_task: None,
            scrollbar_drag: None,
            completion: None,
            completion_task: None,
            git_base: None,
            git_hunks: Vec::new(),
            git_base_task: None,
            git_diff_task: None,
            assist: None,
        };
        editor.rehighlight();
        editor
    }

    /// Opens `path`, or an empty buffer that will be saved there if it doesn't exist yet.
    pub fn open(path: PathBuf, lsp: Option<Entity<LspStore>>, cx: &mut Context<Self>) -> Self {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut editor = Self::new(Buffer::from_text(&text), Some(path), cx);
        editor.reload_git_base(cx);
        if let Some(lsp) = lsp {
            editor.attach_lsp(lsp, cx);
        }
        editor
    }

    /// Picks up a change made to the file outside Null. Unsaved edits are never
    /// overwritten; the reload itself can be undone.
    pub fn reload_from_disk(&mut self, cx: &mut Context<Self>) {
        let Some(path) = &self.path else { return };
        let Ok(text) = std::fs::read_to_string(path) else { return };
        if text == self.buffer.to_string() {
            return;
        }
        if self.buffer.is_dirty() {
            cx.emit(EditorEvent::ChangedOnDisk);
            return;
        }
        let (line, column) = self.caret_point();
        self.record_undo(EditKind::Other);
        self.buffer.replace(0..self.buffer.len_chars(), &text);
        self.buffer.mark_saved();
        self.selection = Selection::caret(self.buffer.offset(line, column));
        self.text_changed(cx);
        cx.notify();
    }

    /// Gives the buffer a new file (after a rename, or the first save of an untitled file).
    pub fn set_path(&mut self, path: PathBuf, lsp: Option<Entity<LspStore>>, cx: &mut Context<Self>) {
        self.release_lsp(cx);
        self.highlighter = languages::for_path(&path).and_then(Highlighter::new);
        self.spans.clear();
        self.path = Some(path);
        self.rehighlight();
        if let Some(lsp) = lsp {
            self.attach_lsp(lsp, cx);
        }
        self.reload_git_base(cx);
        cx.notify();
    }

    pub fn file_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn language(&self) -> Option<&'static languages::Language> {
        languages::for_path(self.path.as_ref()?)
    }

    pub fn language_name(&self) -> &'static str {
        self.language().map_or("Plain Text", |l| l.name)
    }

    /// Zero-based line and column of the caret.
    pub fn caret_point(&self) -> (usize, usize) {
        self.buffer.point(self.selection.head)
    }

    pub fn scrollbar_dragging(&self) -> bool {
        self.scrollbar_drag.is_some()
    }

    pub fn line_height(&self) -> Pixels {
        (self.font_size * 1.7).round()
    }

    /// Call after every change to the text.
    fn text_changed(&mut self, cx: &mut Context<Self>) {
        self.rehighlight();
        self.sync_lsp(cx);
        self.text_changed_for_git(cx);
        self.close_hover(cx);
    }

    /// Brings everything derived from the text up to date after it changes.
    fn rehighlight(&mut self) {
        if let Some(highlighter) = &mut self.highlighter {
            self.spans = highlighter.highlight(&self.buffer.to_string());
        }
        self.refresh_search();
    }

    // ---------- find & replace ----------

    pub fn set_search(&mut self, query: SearchQuery, cx: &mut Context<Self>) {
        if !query.text.is_empty() {
            self.last_query = query.clone();
        }
        let regex = query.build().ok();
        self.search = Some(SearchState { query, regex, matches: Vec::new(), current: None });
        self.refresh_search();
        self.select_current_match(cx);
        cx.notify();
    }

    fn refresh_search(&mut self) {
        let Some(search) = &mut self.search else { return };
        let Some(regex) = &search.regex else {
            search.matches.clear();
            search.current = None;
            return;
        };
        let text = self.buffer.to_string();
        let rope = self.buffer.rope();
        search.matches = search
            .query
            .find_all(regex, &text)
            .into_iter()
            .map(|r| rope.byte_to_char(r.start)..rope.byte_to_char(r.end))
            .collect();
        let from = self.selection.range().start;
        search.current =
            search.matches.iter().position(|m| m.start >= from).or((!search.matches.is_empty()).then_some(0));
    }

    fn select_current_match(&mut self, cx: &mut Context<Self>) {
        let Some(range) = self.search.as_ref().and_then(|s| s.matches.get(s.current?)).cloned() else { return };
        self.selection = Selection { anchor: range.start, head: range.end };
        self.goal_column = None;
        self.touch(cx);
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.search.is_none() && !self.last_query.text.is_empty() {
            self.set_search(self.last_query.clone(), cx);
            return;
        }
        let selection = self.selection.range();
        let Some(search) = &mut self.search else { return };
        if search.matches.is_empty() {
            return;
        }
        let last = search.matches.len() - 1;
        search.current = Some(if forward {
            search.matches.iter().position(|m| m.start >= selection.end && *m != selection).unwrap_or(0)
        } else {
            search.matches.iter().rposition(|m| m.end <= selection.start && *m != selection).unwrap_or(last)
        });
        self.select_current_match(cx);
    }

    pub fn select_next_match(&mut self, cx: &mut Context<Self>) {
        self.step_match(true, cx);
    }

    pub fn select_previous_match(&mut self, cx: &mut Context<Self>) {
        self.step_match(false, cx);
    }

    /// Replaces the selected match (if the selection is one), then moves to the next.
    pub fn replace_next_match(&mut self, replacement: &str, cx: &mut Context<Self>) {
        let selection = self.selection.range();
        let Some(search) = &self.search else { return };
        if let (Some(regex), true) = (&search.regex, search.matches.contains(&selection)) {
            let text = self.buffer.to_string();
            let rope = self.buffer.rope();
            let bytes = rope.char_to_byte(selection.start)..rope.char_to_byte(selection.end);
            let new_text = search.query.replacement_for(regex, &text, bytes, replacement);
            self.edit(selection, &new_text, EditKind::Other, cx);
        }
        self.step_match(true, cx);
    }

    /// Replaces every match as a single undo step.
    pub fn replace_all_matches(&mut self, replacement: &str, cx: &mut Context<Self>) {
        let Some(search) = &self.search else { return };
        let Some(regex) = &search.regex else { return };
        if search.matches.is_empty() {
            return;
        }
        let text = self.buffer.to_string();
        let rope = self.buffer.rope();
        let edits: Vec<(Range<usize>, String)> = search
            .matches
            .iter()
            .map(|m| {
                let bytes = rope.char_to_byte(m.start)..rope.char_to_byte(m.end);
                (m.clone(), search.query.replacement_for(regex, &text, bytes, replacement))
            })
            .collect();
        self.record_undo(EditKind::Other);
        for (range, new_text) in edits.iter().rev() {
            self.buffer.replace(range.clone(), new_text);
        }
        self.selection = Selection::caret(edits[0].0.start);
        self.marked = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    fn deploy_find_bar(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.buffer.slice(self.selection.range());
        let prefill = (!selected.is_empty() && !selected.contains('\n')).then_some(selected);
        if let Some(bar) = self.find_bar.clone() {
            bar.update(cx, |bar, cx| bar.show(prefill, replace, window, cx));
            return;
        }
        let query =
            SearchQuery { text: prefill.unwrap_or_else(|| self.last_query.text.clone()), ..self.last_query.clone() };
        let editor = cx.entity().downgrade();
        let bar = cx.new(|cx| FindBar::new(editor, &query, replace, cx));
        bar.update(cx, |bar, cx| bar.show(None, replace, window, cx));
        self.find_bar = Some(bar);
        self.set_search(query, cx);
    }

    /// Selects a match found by project search and highlights the query's other matches.
    pub fn reveal_match(&mut self, line: usize, columns: Range<usize>, query: SearchQuery, cx: &mut Context<Self>) {
        let start = self.buffer.offset(line, columns.start);
        let end = self.buffer.offset(line, columns.end);
        self.selection = Selection { anchor: start, head: end };
        self.set_search(query, cx);
    }

    pub fn selected_text(&self) -> String {
        self.buffer.slice(self.selection.range())
    }

    /// Closes the find bar, keeps the current match selected, and returns to the text.
    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_bar = None;
        self.search = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn deploy_find(&mut self, _: &DeployFind, window: &mut Window, cx: &mut Context<Self>) {
        self.deploy_find_bar(false, window, cx);
    }

    fn deploy_replace(&mut self, _: &DeployReplace, window: &mut Window, cx: &mut Context<Self>) {
        self.deploy_find_bar(true, window, cx);
    }

    fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.select_next_match(cx);
    }

    fn find_previous(&mut self, _: &FindPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.select_previous_match(cx);
    }

    /// Escape: closes the find bar or clears search highlights, otherwise collapses the selection.
    fn escape(&mut self, _: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
        if self.hover.is_some() {
            self.close_hover(cx);
        } else if self.find_bar.is_some() || self.search.is_some() {
            self.close_find(window, cx);
        } else if !self.selection.is_empty() {
            self.selection = Selection::caret(self.selection.head);
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    /// Marks the caret as just used: it stops blinking and the view follows it.
    fn touch(&mut self, cx: &mut Context<Self>) {
        if self.hover_from_keyboard {
            self.close_hover(cx);
        }
        self.last_activity = Instant::now();
        self.autoscroll = true;
        cx.notify();
    }

    /// Repaints after `delay`, replacing any earlier request. Used for caret blinking.
    pub fn wake_after(&mut self, delay: Duration, cx: &mut Context<Self>) {
        self.wake_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        }));
    }

    // ---------- selection & movement ----------

    fn move_head(&mut self, offset: usize, select: bool, cx: &mut Context<Self>) {
        self.close_completion(cx);
        let offset = offset.min(self.buffer.len_chars());
        if select {
            self.selection.head = offset;
        } else {
            self.selection = Selection::caret(offset);
        }
        self.goal_column = None;
        self.touch(cx);
    }

    fn left_of(&self, offset: usize) -> usize {
        let (line, col) = self.buffer.point(offset);
        match (line, col) {
            (0, 0) => 0,
            (_, 0) => self.buffer.offset(line - 1, usize::MAX),
            _ => offset - 1,
        }
    }

    fn right_of(&self, offset: usize) -> usize {
        let (line, col) = self.buffer.point(offset);
        if col >= self.buffer.line_len(line) {
            if line + 1 < self.buffer.len_lines() { self.buffer.offset(line + 1, 0) } else { offset }
        } else {
            offset + 1
        }
    }

    fn word_left_of(&self, offset: usize) -> usize {
        let mut i = offset;
        while i > 0 && self.buffer.char_at(i - 1).is_some_and(|c| c.is_whitespace()) {
            i -= 1;
        }
        let Some(class) = i.checked_sub(1).and_then(|j| self.buffer.char_at(j)).map(char_class) else {
            return i;
        };
        while i > 0 && self.buffer.char_at(i - 1).map(char_class) == Some(class) {
            i -= 1;
        }
        i
    }

    fn word_right_of(&self, offset: usize) -> usize {
        let len = self.buffer.len_chars();
        let mut i = offset;
        while i < len && self.buffer.char_at(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        let Some(class) = self.buffer.char_at(i).map(char_class) else {
            return i;
        };
        while i < len && self.buffer.char_at(i).map(char_class) == Some(class) {
            i += 1;
        }
        i
    }

    fn word_at(&self, offset: usize) -> Range<usize> {
        let class_at = |i: usize| self.buffer.char_at(i).map(char_class);
        // Clicking just after a word (on its right edge) selects that word.
        let offset =
            if class_at(offset) != Some(CharClass::Word) && offset > 0 && class_at(offset - 1) == Some(CharClass::Word)
            {
                offset - 1
            } else {
                offset
            };
        let class = class_at(offset);
        let mut start = offset;
        let mut end = offset;
        while start > 0 && class_at(start - 1) == class {
            start -= 1;
        }
        while end < self.buffer.len_chars() && class_at(end) == class {
            end += 1;
        }
        start..end
    }

    fn line_start_smart(&self, offset: usize) -> usize {
        let (line, col) = self.buffer.point(offset);
        let indent = indent_len(&self.buffer.line_text(line));
        self.buffer.offset(line, if col == indent { 0 } else { indent })
    }

    fn move_vertically(&mut self, lines: isize, select: bool, cx: &mut Context<Self>) {
        self.close_completion(cx);
        let (line, col) = self.buffer.point(self.selection.head);
        if !select && !self.selection.is_empty() && lines.abs() == 1 {
            let edge = if lines < 0 { self.selection.range().start } else { self.selection.range().end };
            self.selection = Selection::caret(edge);
        }
        let goal = self.goal_column.unwrap_or(col);
        let last = self.buffer.len_lines() - 1;
        let target = line as isize + lines;
        let offset = if target < 0 {
            0
        } else if target as usize > last {
            self.buffer.len_chars()
        } else {
            self.buffer.offset(target as usize, goal)
        };
        if select {
            self.selection.head = offset;
        } else {
            self.selection = Selection::caret(offset);
        }
        self.goal_column = Some(goal);
        self.touch(cx);
    }

    fn page_lines(&self) -> isize {
        self.layout
            .as_ref()
            .map(|l| (l.text_bounds.size.height / l.line_height).floor() as isize - 2)
            .unwrap_or(30)
            .max(1)
    }

    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target =
            if self.selection.is_empty() { self.left_of(self.selection.head) } else { self.selection.range().start };
        self.move_head(target, false, cx);
    }

    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        let target =
            if self.selection.is_empty() { self.right_of(self.selection.head) } else { self.selection.range().end };
        self.move_head(target, false, cx);
    }

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1, false, cx);
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1, false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.left_of(self.selection.head), true, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.right_of(self.selection.head), true, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1, true, cx);
    }

    fn move_word_left(&mut self, _: &MoveWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.word_left_of(self.selection.head), false, cx);
    }

    fn move_word_right(&mut self, _: &MoveWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.word_right_of(self.selection.head), false, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.word_left_of(self.selection.head), true, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.word_right_of(self.selection.head), true, cx);
    }

    fn move_line_start(&mut self, _: &MoveLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.line_start_smart(self.selection.head), false, cx);
    }

    fn move_line_end(&mut self, _: &MoveLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let (line, _) = self.caret_point();
        self.move_head(self.buffer.offset(line, usize::MAX), false, cx);
    }

    fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.line_start_smart(self.selection.head), true, cx);
    }

    fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let (line, _) = self.caret_point();
        self.move_head(self.buffer.offset(line, usize::MAX), true, cx);
    }

    fn move_doc_start(&mut self, _: &MoveDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(0, false, cx);
    }

    fn move_doc_end(&mut self, _: &MoveDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.buffer.len_chars(), false, cx);
    }

    fn select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(0, true, cx);
    }

    fn select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_head(self.buffer.len_chars(), true, cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-self.page_lines(), false, cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(self.page_lines(), false, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = Selection { anchor: 0, head: self.buffer.len_chars() };
        self.last_activity = Instant::now();
        cx.notify();
    }

    // ---------- editing ----------

    fn record_undo(&mut self, kind: EditKind) {
        let now = Instant::now();
        let grouped = matches!(self.last_edit, Some((last, at)) if last == kind && kind != EditKind::Other && now - at < UNDO_GROUP);
        if !grouped {
            self.undo_stack.push(Snapshot { text: self.buffer.rope().clone(), selection: self.selection });
            if self.undo_stack.len() > 1000 {
                self.undo_stack.remove(0);
            }
        }
        self.redo_stack.clear();
        self.last_edit = Some((kind, now));
    }

    fn edit(&mut self, range: Range<usize>, text: &str, kind: EditKind, cx: &mut Context<Self>) {
        if kind == EditKind::Other {
            self.close_completion(cx);
        }
        self.record_undo(kind);
        let end = self.buffer.replace(range, text);
        self.selection = Selection::caret(end);
        self.marked = None;
        self.goal_column = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    fn delete_or(&mut self, range: impl FnOnce(&Self) -> Range<usize>, cx: &mut Context<Self>) {
        let range = if self.selection.is_empty() { range(self) } else { self.selection.range() };
        if !range.is_empty() {
            self.edit(range, "", EditKind::Deleting, cx);
            self.completion_after_delete(cx);
        }
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.empty_pair_around_caret() {
            let head = self.selection.head;
            return self.edit(head - 1..head + 1, "", EditKind::Deleting, cx);
        }
        self.delete_or(
            |this| {
                let head = this.selection.head;
                let (line, col) = this.buffer.point(head);
                let before = this.buffer.line_text(line).chars().take(col).collect::<String>();
                if col > 0 && before.chars().all(|c| c == ' ') {
                    let stop = (col - 1) / TAB_SIZE * TAB_SIZE;
                    return head - (col - stop)..head;
                }
                this.left_of(head)..head
            },
            cx,
        );
    }

    fn backspace_word(&mut self, _: &BackspaceWord, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_or(|this| this.word_left_of(this.selection.head)..this.selection.head, cx);
    }

    fn backspace_line(&mut self, _: &BackspaceLine, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_or(
            |this| {
                let (line, _) = this.caret_point();
                this.buffer.line_to_char(line)..this.selection.head
            },
            cx,
        );
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_or(|this| this.selection.head..this.right_of(this.selection.head), cx);
    }

    fn delete_word(&mut self, _: &DeleteWord, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_or(|this| this.selection.head..this.word_right_of(this.selection.head), cx);
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection.range();
        let (line, _) = self.buffer.point(range.start);
        let indent: String = self.buffer.line_text(line).chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let before = range.start.checked_sub(1).and_then(|i| self.buffer.char_at(i));
        let after = self.buffer.char_at(range.end);
        let opens = matches!(before, Some('{' | '(' | '['));
        let inner = format!("{indent}{}", if opens { " ".repeat(TAB_SIZE) } else { String::new() });
        if opens && matches!((before, after), (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']')))
        {
            let text = format!("\n{inner}\n{indent}");
            let caret = range.start + 1 + inner.chars().count();
            self.edit(range, &text, EditKind::Other, cx);
            self.selection = Selection::caret(caret);
        } else {
            self.edit(range, &format!("\n{inner}"), EditKind::Other, cx);
        }
    }

    fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        self.tab_key(cx);
    }

    pub(crate) fn tab_key(&mut self, cx: &mut Context<Self>) {
        // With anything selected, Tab indents the selected lines (Shift+Tab outdents).
        // This is the easy way on every keyboard layout; Cmd+] / Cmd+[ need [ and ],
        // which on many non-US Mac keyboards take several keys already.
        if !self.selection.is_empty() {
            return self.indent_lines(cx);
        }
        let range = self.selection.range();
        let (_, col) = self.buffer.point(range.start);
        let spaces = TAB_SIZE - col % TAB_SIZE;
        self.edit(range, &" ".repeat(spaces), EditKind::Typing, cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buffer.slice(self.selection.range())));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buffer.slice(self.selection.range())));
            self.edit(self.selection.range(), "", EditKind::Other, cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = text.replace("\r\n", "\n");
            self.edit(self.selection.range(), &text, EditKind::Other, cx);
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.step_history(true, cx);
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.step_history(false, cx);
    }

    fn step_history(&mut self, undo: bool, cx: &mut Context<Self>) {
        let (from, to) = if undo {
            (&mut self.undo_stack, &mut self.redo_stack)
        } else {
            (&mut self.redo_stack, &mut self.undo_stack)
        };
        let Some(snapshot) = from.pop() else { return };
        to.push(Snapshot { text: self.buffer.rope().clone(), selection: self.selection });
        self.buffer.restore(snapshot.text);
        self.selection = snapshot.selection;
        self.last_edit = None;
        self.marked = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save_to_disk(cx);
    }

    /// Writes the buffer to its file. Returns false if there's no file or writing failed.
    pub fn save_to_disk(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(path) = &self.path else {
            cx.emit(EditorEvent::NeedsPath);
            return false;
        };
        match std::fs::write(path, self.buffer.to_string()) {
            Ok(()) => {
                self.buffer.mark_saved();
                self.lsp_saved(cx);
                cx.emit(EditorEvent::Saved);
                cx.notify();
                true
            }
            Err(err) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                cx.emit(EditorEvent::SaveFailed(format!("Couldn't save {name}: {err}")));
                false
            }
        }
    }

    pub fn set_font_size(&mut self, size: Pixels, cx: &mut Context<Self>) {
        if size != self.font_size {
            self.font_size = size;
            self.caret.placed = false;
            self.touch(cx);
        }
    }

    fn render_completions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.completion.as_ref()?;
        let bounds = self.caret_bounds(self.selection.head)?;
        let theme = cx.global::<Theme>();
        let code_font = cx.global::<Fonts>().code.clone();
        let rows = (0..menu.shown.len()).map(|ix| {
            let suggestion = menu.suggestion(ix);
            let selected = ix == menu.selected;
            let (badge, color) = completion_badge(suggestion.kind, theme);
            let highlight = HighlightStyle { color: Some(theme.caret), ..Default::default() };
            let label =
                StyledText::new(suggestion.label.clone()).with_highlights(menu.shown[ix].1.iter().filter_map(|&b| {
                    let len = suggestion.label.get(b..)?.chars().next()?.len_utf8();
                    Some((b..b + len, highlight))
                }));
            div()
                .id(ix)
                .h(px(26.))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .rounded(px(6.))
                .when(selected, |row| row.bg(theme.accent_soft))
                .child(div().w(px(14.)).flex_none().text_color(color).text_size(px(11.)).child(badge))
                .child(div().flex_none().text_color(theme.foreground).child(label))
                .children(suggestion.detail.clone().map(|detail| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_right()
                        .text_size(px(11.5))
                        .text_color(theme.faint)
                        .child(detail)
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.accept_completion(ix, cx)))
        });
        let list = div()
            .id("completions")
            .occlude()
            .track_scroll(&menu.scroll)
            .overflow_y_scroll()
            .min_w(px(260.))
            .max_w(px(520.))
            .max_h(px(26. * 8. + 10.))
            .p(px(4.))
            .rounded(px(9.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.hairline)
            .shadow_lg()
            .font_family(code_font)
            .text_size(px(13.))
            .children(rows);
        Some(
            deferred(
                anchored()
                    .position(point(bounds.left() - px(30.), bounds.bottom() + px(4.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .into_any_element(),
        )
    }

    fn render_hover(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = self.hover.as_ref()?;
        let bounds = self.caret_bounds(card.range.start)?;
        let theme = cx.global::<Theme>();
        let code_font = cx.global::<Fonts>().code.clone();
        let diagnostics = card.diagnostics.iter().map(|(severity, message)| {
            let color = if *severity == lsp_types::DiagnosticSeverity::ERROR {
                theme.error
            } else if *severity == lsp_types::DiagnosticSeverity::WARNING {
                theme.warning
            } else {
                theme.muted
            };
            div().text_color(color).child(message.clone())
        });
        let blocks = card.blocks.iter().map(|block| {
            if block.code {
                div()
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .bg(theme.background)
                    .font_family(code_font.clone())
                    .text_size(px(12.5))
                    .text_color(theme.foreground)
                    .child(block.text.clone())
            } else {
                div().text_color(theme.muted).child(block.text.clone())
            }
        });
        let card = div()
            .id("hover-card")
            .occlude()
            .on_hover(cx.listener(|this, inside: &bool, _, cx| this.set_mouse_in_card(*inside, cx)))
            .max_w(px(560.))
            .max_h(px(340.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(10.))
            .rounded(px(10.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.hairline)
            .shadow_lg()
            .text_size(px(13.))
            .line_height(px(19.))
            .children(diagnostics)
            .children(blocks);
        Some(
            deferred(
                anchored()
                    .position(point(bounds.left(), bounds.bottom() + px(6.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(card),
            )
            .into_any_element(),
        )
    }

    // ---------- mouse ----------

    /// Char offset under a window position, using last frame's layout.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        let Some(layout) = &self.layout else { return self.selection.head };
        let y = (position.y - layout.text_origin.y) / layout.line_height;
        if y < 0. {
            return 0;
        }
        let line = y.floor() as usize;
        if line >= self.buffer.len_lines() {
            return self.buffer.len_chars();
        }
        let x = position.x - layout.text_origin.x;
        let col = if layout.visible_lines.contains(&line) {
            let shaped = &layout.shaped[line - layout.visible_lines.start];
            let byte = shaped.closest_index_for_x(x);
            self.buffer.line_text(line)[..byte].chars().count()
        } else {
            (x / layout.char_width).round().max(0.) as usize
        };
        self.buffer.offset(line, col)
    }

    fn line_range(&self, offset: usize) -> Range<usize> {
        let (line, _) = self.buffer.point(offset);
        let end = if line + 1 < self.buffer.len_lines() {
            self.buffer.line_to_char(line + 1)
        } else {
            self.buffer.len_chars()
        };
        self.buffer.line_to_char(line)..end
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        self.close_hover(cx);
        self.close_completion(cx);
        if self.scrollbar_mouse_down(event.position, cx) {
            return;
        }
        let offset = self.offset_at(event.position);
        let unit = match event.click_count {
            2 => DragUnit::Word(self.word_at(offset)),
            3 => DragUnit::Line(self.line_range(offset)),
            _ => DragUnit::Char,
        };
        match &unit {
            DragUnit::Word(range) | DragUnit::Line(range) => {
                self.selection = Selection { anchor: range.start, head: range.end }
            }
            DragUnit::Char if event.modifiers.shift => self.selection.head = offset,
            DragUnit::Char => self.selection = Selection::caret(offset),
        }
        self.goal_column = None;
        self.dragging = Some(unit);
        self.touch(cx);
        if event.modifiers.secondary() && event.click_count == 1 {
            self.dragging = None;
            self.go_to_definition_at(offset, cx);
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let was_over_scrollbar = self.over_scrollbar();
        self.mouse_position = Some(event.position);
        if self.scrollbar_drag.is_some() && event.pressed_button == Some(MouseButton::Left) {
            return self.scrollbar_drag_to(event.position, cx);
        }
        if was_over_scrollbar != self.over_scrollbar() {
            cx.notify();
        }
        if self.alt_held || self.secondary_held || self.link_word.is_some() {
            self.update_hover(cx);
        }
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let offset = self.offset_at(event.position);
        let (origin, target) = match &self.dragging {
            None => return,
            Some(DragUnit::Char) => {
                self.selection.head = offset;
                self.touch(cx);
                return;
            }
            Some(DragUnit::Word(origin)) => (origin.clone(), self.word_at(offset)),
            Some(DragUnit::Line(origin)) => (origin.clone(), self.line_range(offset)),
        };
        // Grow by whole words or lines, keeping the one first clicked selected.
        self.selection = if target.start < origin.start {
            Selection { anchor: origin.end, head: target.start }
        } else {
            Selection { anchor: origin.start, head: target.end.max(origin.end) }
        };
        self.touch(cx);
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.dragging = None;
        if self.scrollbar_drag.take().is_some() {
            cx.notify();
        }
    }

    pub fn over_scrollbar(&self) -> bool {
        let (Some(layout), Some(position)) = (&self.layout, self.mouse_position) else { return false };
        layout.scrollbar.as_ref().is_some_and(|s| s.track.contains(&position))
    }

    /// A press on the scrollbar: grab the thumb, or jump to where the track was clicked.
    fn scrollbar_mouse_down(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(bar) = self.layout.as_ref().and_then(|l| l.scrollbar.as_ref()) else { return false };
        if !bar.track.contains(&position) {
            return false;
        }
        let grab =
            if bar.thumb.contains(&position) { position.y - bar.thumb.top() } else { bar.thumb.size.height / 2. };
        self.scrollbar_drag = Some(grab);
        self.scrollbar_drag_to(position, cx);
        true
    }

    fn scrollbar_drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (Some(grab), Some(bar)) = (self.scrollbar_drag, self.layout.as_ref().and_then(|l| l.scrollbar.as_ref()))
        else {
            return;
        };
        let travel = f32::from(bar.track.size.height - bar.thumb.size.height).max(1.);
        let fraction = (f32::from(position.y - grab - bar.track.top()) / travel).clamp(0., 1.);
        self.scroll.y = fraction * bar.max_scroll;
        self.scroll.target_y = self.scroll.y;
        cx.notify();
    }

    fn on_modifiers_changed(&mut self, event: &ModifiersChangedEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.alt_held = event.modifiers.alt;
        self.secondary_held = event.modifiers.secondary();
        if !self.alt_held {
            self.hover_suppressed = false;
        }
        self.update_hover(cx);
    }

    fn on_key_down(&mut self, _: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.alt_held && !self.hover_suppressed {
            self.hover_suppressed = true;
            self.close_hover(cx);
        }
    }

    fn show_info(&mut self, _: &ShowInfo, _: &mut Window, cx: &mut Context<Self>) {
        self.show_info_at_caret(cx);
    }

    fn show_completions(&mut self, _: &ShowCompletions, _: &mut Window, cx: &mut Context<Self>) {
        self.show_completions_now(cx);
    }

    fn completion_next(&mut self, _: &CompletionNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_completion(1, cx);
    }

    fn completion_previous(&mut self, _: &CompletionPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_completion(-1, cx);
    }

    fn confirm_completion(&mut self, _: &ConfirmCompletion, _: &mut Window, cx: &mut Context<Self>) {
        let selected = self.completion.as_ref().map_or(0, |m| m.selected);
        self.accept_completion(selected, cx);
    }

    fn toggle_comment_action(&mut self, _: &ToggleComment, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_comment(cx);
    }

    fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        self.indent_lines(cx);
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.outdent_lines(cx);
    }

    fn move_line_up(&mut self, _: &MoveLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_lines(false, cx);
    }

    fn move_line_down(&mut self, _: &MoveLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_lines(true, cx);
    }

    fn duplicate_line_up(&mut self, _: &DuplicateLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.duplicate_lines(false, cx);
    }

    fn duplicate_line_down(&mut self, _: &DuplicateLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.duplicate_lines(true, cx);
    }

    fn delete_line(&mut self, _: &DeleteLine, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_lines(cx);
    }

    fn select_line_action(&mut self, _: &SelectLine, _: &mut Window, cx: &mut Context<Self>) {
        self.select_line(cx);
    }

    fn inline_assist(&mut self, _: &InlineAssist, window: &mut Window, cx: &mut Context<Self>) {
        self.open_inline_assist(window, cx);
    }

    fn cancel_completion(&mut self, _: &CancelCompletion, _: &mut Window, cx: &mut Context<Self>) {
        self.close_completion(cx);
    }

    fn go_to_definition(&mut self, _: &GoToDefinition, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_definition_at(self.selection.head, cx);
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let line_height = f32::from(self.line_height());
        match event.delta {
            // Trackpads report exact pixels and already feel smooth: follow them 1:1.
            ScrollDelta::Pixels(delta) => {
                self.scroll.y -= f32::from(delta.y);
                self.scroll.target_y = self.scroll.y;
                self.scroll.x -= f32::from(delta.x);
            }
            // Mouse wheels jump in notches: glide to the new position instead.
            ScrollDelta::Lines(delta) => {
                self.scroll.target_y -= delta.y * line_height * 3.;
                self.scroll.x -= delta.x * line_height * 3.;
            }
        }
        cx.notify();
    }

    /// Screen bounds of the caret position at `offset`, if it was drawn last frame.
    fn caret_bounds(&self, offset: usize) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let (line, col) = self.buffer.point(offset);
        let x = if layout.visible_lines.contains(&line) {
            let text = self.buffer.line_text(line);
            let byte = text.char_indices().nth(col).map_or(text.len(), |(b, _)| b);
            layout.shaped[line - layout.visible_lines.start].x_for_index(byte)
        } else {
            layout.char_width * col as f32
        };
        Some(Bounds::new(
            point(layout.text_origin.x + x, layout.text_origin.y + layout.line_height * line as f32),
            size(px(2.), layout.line_height),
        ))
    }
}

/// What a mouse drag selects by, set by the click that started it.
enum DragUnit {
    Char,
    Word(Range<usize>),
    Line(Range<usize>),
}

/// A one-letter hint of what a suggestion is, in the color code uses for it.
fn completion_badge(kind: Option<lsp_types::CompletionItemKind>, theme: &Theme) -> (&'static str, gpui::Hsla) {
    use crate::theme::Syntax;
    use lsp_types::CompletionItemKind as K;
    match kind {
        Some(K::FUNCTION | K::METHOD | K::CONSTRUCTOR) => ("ƒ", theme.syntax(Syntax::Function)),
        Some(K::STRUCT | K::CLASS | K::TYPE_PARAMETER) => ("S", theme.syntax(Syntax::Type)),
        Some(K::ENUM | K::ENUM_MEMBER) => ("E", theme.syntax(Syntax::Type)),
        Some(K::INTERFACE) => ("T", theme.syntax(Syntax::Type)),
        Some(K::FIELD | K::PROPERTY) => ("·", theme.syntax(Syntax::Property)),
        Some(K::VARIABLE) => ("v", theme.syntax(Syntax::Plain)),
        Some(K::CONSTANT) => ("c", theme.syntax(Syntax::Number)),
        Some(K::MODULE) => ("m", theme.syntax(Syntax::Keyword)),
        Some(K::KEYWORD) => ("k", theme.syntax(Syntax::Keyword)),
        Some(K::SNIPPET) => ("…", theme.muted),
        _ => ("·", theme.muted),
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum CharClass {
    Word,
    Space,
    Punct,
}

fn char_class(c: char) -> CharClass {
    if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else if c.is_whitespace() {
        CharClass::Space
    } else {
        CharClass::Punct
    }
}

fn indent_len(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.buffer.utf16_to_char(range_utf16.start)..self.buffer.utf16_to_char(range_utf16.end);
        actual_range.replace(self.buffer.char_to_utf16(range.start)..self.buffer.char_to_utf16(range.end));
        Some(self.buffer.slice(range))
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let range = self.selection.range();
        Some(UTF16Selection {
            range: self.buffer.char_to_utf16(range.start)..self.buffer.char_to_utf16(range.end),
            reversed: self.selection.head < self.selection.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.buffer.char_to_utf16(r.start)..self.buffer.char_to_utf16(r.end))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| self.buffer.utf16_to_char(r.start)..self.buffer.utf16_to_char(r.end))
            .or(self.marked.clone())
            .unwrap_or(self.selection.range());
        let mut chars = text.chars();
        if let (Some(c), None, None) = (chars.next(), chars.next(), &self.marked)
            && range == self.selection.range()
            && self.type_pair_char(c, cx)
        {
            return;
        }
        self.edit(range, text, EditKind::Typing, cx);
        self.completion_after_typing(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| self.buffer.utf16_to_char(r.start)..self.buffer.utf16_to_char(r.end))
            .or(self.marked.clone())
            .unwrap_or(self.selection.range());
        self.record_undo(EditKind::Typing);
        let end = self.buffer.replace(range.clone(), text);
        self.marked = (!text.is_empty()).then_some(range.start..end);
        let utf16_to_chars = |u: usize| {
            let mut units = 0;
            text.chars()
                .take_while(|c| {
                    units += c.len_utf16();
                    units <= u
                })
                .count()
        };
        self.selection = match new_selected_range_utf16 {
            Some(r) => {
                Selection { anchor: range.start + utf16_to_chars(r.start), head: range.start + utf16_to_chars(r.end) }
            }
            None => Selection::caret(end),
        };
        self.text_changed(cx);
        self.touch(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.caret_bounds(self.buffer.utf16_to_char(range_utf16.start))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.buffer.char_to_utf16(self.offset_at(point)))
    }
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The find bar sits beside the text, not inside the "Editor" key context,
        // so typing in it never triggers editor shortcuts.
        let find_bar = self.find_bar.clone();
        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("Editor");
        if self.completion.is_some() {
            key_context.add("showing_completions");
        }
        let text = div()
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            .size_full()
            .cursor(if self.link_word.is_some() { CursorStyle::PointingHand } else { CursorStyle::IBeam })
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::move_word_left))
            .on_action(cx.listener(Self::move_word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::move_line_start))
            .on_action(cx.listener(Self::move_line_end))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::move_doc_start))
            .on_action(cx.listener(Self::move_doc_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::backspace_word))
            .on_action(cx.listener(Self::backspace_line))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::deploy_find))
            .on_action(cx.listener(Self::deploy_replace))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(Self::go_to_definition))
            .on_action(cx.listener(Self::show_info))
            .on_action(cx.listener(Self::show_completions))
            .on_action(cx.listener(Self::completion_next))
            .on_action(cx.listener(Self::completion_previous))
            .on_action(cx.listener(Self::confirm_completion))
            .on_action(cx.listener(Self::cancel_completion))
            .on_action(cx.listener(Self::inline_assist))
            .on_action(cx.listener(Self::toggle_comment_action))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::move_line_up))
            .on_action(cx.listener(Self::move_line_down))
            .on_action(cx.listener(Self::duplicate_line_up))
            .on_action(cx.listener(Self::duplicate_line_down))
            .on_action(cx.listener(Self::delete_line))
            .on_action(cx.listener(Self::select_line_action))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(EditorElement::new(cx.entity()));
        let hover = self.render_hover(cx);
        let completions = self.render_completions(cx);
        let assist = self.render_assist(cx);
        div()
            .relative()
            .size_full()
            .child(text)
            .when_some(find_bar, |editor, bar| editor.child(div().absolute().top(px(8.)).right(px(16.)).child(bar)))
            .children(hover)
            .children(completions)
            .children(assist)
    }
}
