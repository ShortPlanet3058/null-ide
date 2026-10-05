mod assist;
mod breakpoints;
mod changes;
mod commands;
mod completion;
mod cursors;
mod fixes;
mod fold;
mod ghost;
mod hints;
mod intel;
mod marks;
mod refactor;
mod review;
mod signature;
mod snippet;
mod structure;

pub use assist::{Block, BlockKind};
pub use breakpoints::{Breakpoint, ToggleBreakpoint};
pub use completion::CompletionMenu;
pub use cursors::Cursor;
pub use fixes::QuickFix;
pub use fold::{Fold, FoldAll, Unfold, UnfoldAll};
pub use ghost::{AcceptGhost, AcceptGhostLine, AcceptGhostWord, NextGhost};
pub use intel::HoverCard;
pub use refactor::{FindReferences, FormatDocument, FormatSelection, RenameSymbol, apply_edits};
pub use review::{KeepHunk, UndoHunk};
pub use structure::{
    ExpandSelection, GoToMatchingBracket, JoinLines, LowerCase, NewlineAbove, NewlineBelow, NextChange, PreviousChange,
    ShrinkSelection, SortLines, UpperCase,
};

use crate::buffer::Buffer;
use crate::element::{EditorElement, RowLayout};
use crate::file_style::Indent as IndentStyle;
use crate::find_bar::{CloseFind, DeployFind, DeployReplace, FindBar, FindNext, FindPrevious};
use crate::fonts::Fonts;
use crate::highlight::{Highlighter, Span};
use crate::languages;
use crate::lsp_store::LspStore;
use crate::search::SearchQuery;
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui;
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, CursorStyle, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, HighlightStyle, KeyBinding, KeyContext, KeyDownEvent, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, StyledText,
    Subscription, Task, UTF16Selection, Window, actions, anchored, deferred, div, point, prelude::*, px, size,
};
use regex::Regex;
use ropey::Rope;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

actions!(file_style, [IndentWithTabs, IndentWith2Spaces, IndentWith4Spaces, UseLfLineEndings, UseCrlfLineEndings]);

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
        GoToTypeDefinition,
        GoToImplementation,
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
        AddNextOccurrence,
        SelectAllOccurrences,
        AddCursorAbove,
        AddCursorBelow,
        AddCursorsToLineEnds,
        UndoCursor,
    ]
);

pub const TAB_SIZE: usize = 4;
/// Edits of the same kind closer together than this undo as one step.
const UNDO_GROUP: Duration = Duration::from_millis(1000);

/// The keys for the AI's field and changes; registered after every other part's keys.
pub fn bind_refactor_keys(cx: &mut App) {
    refactor::bind_keys(cx);
    fold::bind_keys(cx);
    fixes::bind_keys(cx);
    breakpoints::bind_keys(cx);
    structure::bind_keys(cx);
    snippet::bind_keys(cx);
}

pub fn bind_ai_keys(cx: &mut App) {
    assist::bind_keys(cx);
    review::bind_keys(cx);
    ghost::bind_keys(cx);
}

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
        KeyBinding::new("secondary-f12", GoToImplementation, ctx),
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
        KeyBinding::new("secondary-d", AddNextOccurrence, ctx),
        KeyBinding::new("secondary-shift-l", SelectAllOccurrences, ctx),
        KeyBinding::new("secondary-alt-up", AddCursorAbove, ctx),
        KeyBinding::new("secondary-alt-down", AddCursorBelow, ctx),
        KeyBinding::new("alt-shift-i", AddCursorsToLineEnds, ctx),
        KeyBinding::new("secondary-u", UndoCursor, ctx),
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
    /// The buffer's version for this text: back to the saved one means saved.
    version: u64,
    selection: Selection,
    extra: Vec<Selection>,
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
    /// Rename the symbol at `position` everywhere: the workspace applies it to every file.
    Rename {
        position: lsp_types::Position,
        new_name: String,
    },
    /// Every change of a review was kept or undone.
    Reviewed,
    /// The breakpoints changed (set, removed, or moved by an edit).
    BreakpointsChanged,
    /// Several places to choose from (implementations): the workspace lists them.
    ShowLocations {
        title: String,
        locations: Vec<lsp_types::Location>,
    },
    /// The caret jumped within the file (to a definition): Back comes back to `from`.
    Jumped {
        from: (usize, usize),
    },
    /// A quick fix was picked: the workspace works out its edits and applies them.
    CodeAction(lsp_types::CodeActionOrCommand),
    /// Show where the symbol at `position` is used.
    FindReferences {
        position: lsp_types::Position,
        name: String,
    },
}

impl EventEmitter<EditorEvent> for Editor {}

/// Where things were drawn last frame, for mouse hit testing and IME.
pub struct Layout {
    /// Screen position of line 0, column 0, with scrolling applied.
    pub text_origin: Point<Pixels>,
    pub text_bounds: Bounds<Pixels>,
    /// The whole editor, gutter included.
    pub bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub char_width: Pixels,
    /// The rows on screen (lines, or parts of lines when they wrap), from `first_row`.
    pub first_row: usize,
    pub rows: Vec<RowLayout>,
    /// Lines pinned at the top by sticky scroll: where each is, and which line it is.
    pub sticky: Vec<(Bounds<Pixels>, usize)>,
    /// None when everything fits and there's nothing to scroll.
    pub scrollbar: Option<ScrollbarLayout>,
}

pub struct ScrollbarLayout {
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
    /// How far the view can scroll, in pixels.
    pub max_scroll: f32,
}

/// A file's text and state, to open a second copy of it (see [`Editor::twin`]).
pub struct TwinSource {
    text: String,
    dirty: bool,
    path: PathBuf,
    style: crate::file_style::FileStyle,
    view: (usize, usize, usize),
}

pub struct Scroll {
    pub y: f32,
    pub target_y: f32,
    pub x: f32,
    pub last_tick: Option<Instant>,
    /// The direction a trackpad swipe settled on (true: sideways), until it ends.
    pub swipe_sideways: Option<bool>,
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
    /// How this file is written: its indentation and line endings, kept when editing.
    pub style: crate::file_style::FileStyle,
    /// Colours for the lines around the view (see [`Self::highlight_lines`]).
    pub spans: Vec<Span>,
    /// The longest line's width in columns, for the buffer revision it was measured at.
    longest_line: std::cell::Cell<(u64, usize)>,
    /// The buffer revision and byte range `spans` cover.
    spans_for: Option<(u64, Range<usize>)>,
    /// `problems()` for a (diagnostics version, buffer revision).
    problems_cache: std::cell::RefCell<Option<((u64, u64), std::rc::Rc<Vec<intel::Problem>>)>>,
    pinned: std::cell::RefCell<intel::Pinned>,
    /// The main cursor: the one the view follows. Any others are in `extra`.
    pub selection: Selection,
    goal_column: Option<usize>,
    pub extra: Vec<Cursor>,
    batch: Option<cursors::Batch>,
    /// The word ⌘D picked from a bare caret: its occurrences must be whole words too.
    word_pick: Option<Range<usize>>,
    occurrence_whole_word: bool,
    pub marked: Option<Range<usize>>,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    /// The last edit, for grouping undo steps: its kind, when, and where the caret ended.
    last_edit: Option<(EditKind, Instant, usize)>,
    pub scroll: Scroll,
    pub caret: CaretMotion,
    /// Last caret move or edit. The caret stays solid until it has been idle a moment.
    pub last_activity: Instant,
    wake_task: Option<Task<()>>,
    pub layout: Option<Layout>,
    /// Which screen rows each line takes; kept up to date when drawing.
    pub wrap: crate::wrap::WrapMap,
    pub autoscroll: bool,
    /// Set by the mouse: bring the caret into view without the keyboard's margin, so a
    /// click near an edge doesn't scroll the text under the pointer.
    pub reveal_only: bool,
    dragging: Option<DragUnit>,
    pub font_size: Pixels,
    pub search: Option<SearchState>,
    find_bar: Option<Entity<FindBar>>,
    /// Reused by Find Next when the find bar is closed, and to prefill it.
    last_query: SearchQuery,
    lsp: Option<Entity<LspStore>>,
    lsp_version: i32,
    /// The buffer revision the language server has.
    lsp_revision: u64,
    /// A second copy of a file open on both sides: it leaves talking to the language
    /// server to the first, so the server doesn't hear every change twice.
    pub lsp_follower: bool,
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
    pub(crate) mouse_position: Option<Point<Pixels>>,
    definition_task: Option<Task<()>>,
    /// While dragging the scrollbar: where on the thumb it was grabbed.
    scrollbar_drag: Option<Pixels>,
    pub completion: Option<CompletionMenu>,
    completion_task: Option<Task<()>>,
    /// ⌘.'s list of quick fixes.
    fix_menu: Option<fixes::FixMenu>,
    /// Type hints from the language server.
    hints: hints::Hints,
    /// Other uses of the symbol at the caret.
    symbol_marks: marks::SymbolMarks,
    /// Lines (from 0) where the debugger should stop; they move with edits.
    pub breakpoints: Vec<usize>,
    /// Breakpoints that only stop when something holds: (line, condition).
    pub breakpoint_conditions: Vec<(usize, String)>,
    breakpoints_revision: u64,
    /// The field a breakpoint's condition is being typed in.
    editing_condition: Option<breakpoints::ConditionEdit>,
    /// The line the debugger stopped on, while it's stopped in this file.
    pub execution_line: Option<usize>,
    /// While stopped: values of the variables the lines above name, shown faintly at
    /// their end (line, text).
    pub inline_values: Vec<(usize, String)>,
    /// While stopped in this file: the call's variables, for the info card (⌥ over a name).
    pub debug_locals: Vec<(String, String)>,
    /// The snippet being filled in, if any.
    snippet: Option<snippet::Session>,
    /// Cursors as they were before each one was added, for ⌘U.
    cursor_history: Vec<(Selection, Vec<Cursor>)>,
    /// ⌃⇧⌘→'s steps, to shrink back through.
    expansions: structure::Expansions,
    fixes_task: Option<Task<()>>,
    /// Parameter hints while typing a call.
    signature: signature::Signature,
    folds: fold::Folds,
    /// The file as last committed, to mark changed lines in the gutter.
    git_base: Option<std::sync::Arc<str>>,
    pub git_hunks: Vec<crate::git::Hunk>,
    git_base_task: Option<Task<()>>,
    /// Who last changed the caret's line, once it rests there.
    blame: Option<changes::Blame>,
    blame_task: Option<Task<()>>,
    git_diff_task: Option<Task<()>>,
    /// ⌘I: the field while it's open, a change until it's kept or undone, an answer.
    prompt: Option<assist::Prompting>,
    ai_change: Option<assist::Change>,
    /// An AI task's changes to this file, being reviewed.
    review: Option<review::Review>,
    note: Option<assist::Note>,
    ghost: Option<ghost::Ghost>,
    ghost_task: Option<Task<()>>,
    ghost_cache: Vec<(String, Vec<String>)>,
    renaming: Option<refactor::Renaming>,
    format_task: Option<Task<()>>,
    /// Rows between the lines for those, rebuilt as they change.
    pub blocks: Vec<Block>,
}

impl Editor {
    pub fn new(buffer: Buffer, path: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let highlighter = path.as_deref().and_then(languages::for_path).and_then(Highlighter::new);
        let style = crate::file_style::FileStyle::for_file(
            path.as_deref(),
            &buffer.slice(0..buffer.len_chars().min(200_000)),
            cx.global::<Settings>().default_indent(),
        );
        let mut editor = Self {
            focus_handle: cx.focus_handle(),
            buffer,
            path,
            highlighter,
            style,
            spans: Vec::new(),
            spans_for: None,
            longest_line: std::cell::Cell::new((u64::MAX, 0)),
            problems_cache: Default::default(),
            pinned: Default::default(),
            selection: Selection::caret(0),
            goal_column: None,
            extra: Vec::new(),
            batch: None,
            word_pick: None,
            occurrence_whole_word: false,
            marked: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit: None,
            scroll: Scroll { y: 0., target_y: 0., x: 0., last_tick: None, swipe_sideways: None },
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
            wrap: Default::default(),
            autoscroll: false,
            reveal_only: false,
            dragging: None,
            font_size: px(cx.global::<Settings>().font_size),
            search: None,
            find_bar: None,
            last_query: SearchQuery::default(),
            lsp: None,
            lsp_version: 0,
            lsp_revision: 0,
            lsp_follower: false,
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
            fix_menu: None,
            expansions: Default::default(),
            cursor_history: Vec::new(),
            snippet: None,
            breakpoints: Vec::new(),
            breakpoint_conditions: Vec::new(),
            breakpoints_revision: 0,
            editing_condition: None,
            execution_line: None,
            inline_values: Vec::new(),
            debug_locals: Vec::new(),
            symbol_marks: Default::default(),
            hints: Default::default(),
            fixes_task: None,
            signature: Default::default(),
            folds: Default::default(),
            git_base: None,
            git_hunks: Vec::new(),
            git_base_task: None,
            blame: None,
            blame_task: None,
            git_diff_task: None,
            prompt: None,
            ai_change: None,
            review: None,
            note: None,
            ghost: None,
            ghost_task: None,
            ghost_cache: Vec::new(),
            renaming: None,
            format_task: None,
            blocks: Vec::new(),
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
        self.single_cursor();
        self.selection = Selection::caret(self.buffer.offset(line, column));
        self.text_changed(cx);
        cx.notify();
    }

    /// Gives the buffer a new file (after a rename, or the first save of an untitled file).
    pub fn set_path(&mut self, path: PathBuf, lsp: Option<Entity<LspStore>>, cx: &mut Context<Self>) {
        self.release_lsp(cx);
        self.highlighter = languages::for_path(&path).and_then(Highlighter::new);
        self.spans.clear();
        self.spans_for = None;
        // A new name can bring other rules (.editorconfig sections, Go's tabs).
        self.style = crate::file_style::FileStyle::for_file(
            Some(&path),
            &self.buffer.slice(0..self.buffer.len_chars().min(200_000)),
            cx.global::<Settings>().default_indent(),
        );
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
            .unwrap_or_else(|| "Untitled".into())
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

    /// Brings back unsaved text kept from last time: the whole text replaced, as one edit
    /// that can be undone, and the file unsaved.
    pub fn restore_unsaved(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.buffer.to_string() == text {
            return;
        }
        let caret = self.caret_point();
        let all = 0..self.buffer.len_chars();
        self.edit(all, text, EditKind::Other, cx);
        self.set_caret_point(caret, cx);
    }

    /// Puts the caret at a line and column (kept within the text), and shows it.
    pub fn set_caret_point(&mut self, (line, column): (usize, usize), cx: &mut Context<Self>) {
        let line = line.min(self.buffer.len_lines().saturating_sub(1));
        let column = column.min(self.buffer.line_len(line));
        self.single_cursor();
        self.selection = Selection::caret(self.buffer.offset(line, column));
        self.goal_column = None;
        self.touch(cx);
    }

    pub fn scrollbar_dragging(&self) -> bool {
        self.scrollbar_drag.is_some()
    }

    pub fn line_height(&self) -> Pixels {
        (self.font_size * 1.7).round()
    }

    /// Call after every change to the text.
    fn text_changed(&mut self, cx: &mut Context<Self>) {
        // Running once per cursor: catch up once at the end.
        if let Some(batch) = &mut self.batch {
            batch.changed = true;
            return;
        }
        self.rehighlight();
        self.folds_after_edit(cx);
        self.ai_text_changed();
        self.sync_lsp(cx);
        self.text_changed_for_git(cx);
        self.hints_after_edit();
        self.breakpoints_after_edit(cx);
        // Cursors from before an edit aren't somewhere to go back to.
        self.cursor_history.clear();
        self.close_hover(cx);
        self.close_fixes(cx);
    }

    /// Brings everything derived from the text up to date after it changes. Colours
    /// follow when the lines are next drawn.
    fn rehighlight(&mut self) {
        self.refresh_search();
    }

    /// The widest line in columns (tabs at full width). Measured over the whole file once;
    /// after typing, only the edited lines, so it can stay a little wide after a long line
    /// shrinks (some extra room to scroll) but never costs a pass over the file.
    pub(crate) fn longest_line(&self) -> usize {
        let width = |line: ropey::RopeSlice| {
            line.chars().filter(|c| *c != '\n' && *c != '\r').fold(0, |col, c| col + crate::wrap::char_columns(c, col))
        };
        let (revision, cols) = self.longest_line.get();
        if revision == self.buffer.revision() {
            return cols;
        }
        let rope = self.buffer.rope();
        let last = self.buffer.len_lines().saturating_sub(1);
        let cols = match self.buffer.edits_since(revision).filter(|_| revision != u64::MAX) {
            Some(edits) => edits
                .flat_map(|e| e.start.0..=e.new_end.0)
                .map(|line| width(rope.line(line.min(last))))
                .fold(cols, usize::max),
            None => rope.lines().map(width).max().unwrap_or(0),
        };
        self.longest_line.set((self.buffer.revision(), cols));
        cols
    }

    /// Makes sure `spans` colours `lines`: the syntax tree catches up with the edits
    /// (only what changed is parsed again), then the lines around the view are
    /// coloured, with some room so scrolling a little needs nothing new.
    pub(crate) fn highlight_lines(&mut self, lines: Range<usize>) {
        /// Lines coloured beyond the view on each side, so scrolling doesn't colour again…
        const ROOM: usize = 120;
        /// …but after an edit, only a few: what's on screen is what's needed now (colouring
        /// all the room took 3 ms a keystroke on a big file).
        const ROOM_AFTER_EDIT: usize = 10;
        let Some(highlighter) = &mut self.highlighter else { return };
        let revision = self.buffer.revision();
        let wanted = self.buffer.line_to_byte(lines.start)..self.buffer.line_to_byte(lines.end);
        let edited = match &self.spans_for {
            Some((r, range)) if *r == revision => {
                if range.start <= wanted.start && range.end >= wanted.end {
                    return;
                }
                false
            }
            Some(_) => true,
            None => false,
        };
        highlighter.sync(&self.buffer);
        let room = if edited { ROOM_AFTER_EDIT } else { ROOM };
        let range =
            self.buffer.line_to_byte(lines.start.saturating_sub(room))..self.buffer.line_to_byte(lines.end + room);
        self.spans = highlighter.spans(self.buffer.rope(), range.clone());
        self.spans_for = Some((revision, range));
    }

    /// The colours of one line, for a line shown away from the others (pinned at the top).
    pub(crate) fn line_spans(&mut self, line: usize) -> Vec<Span> {
        let start = self.buffer.line_to_byte(line);
        let end = self.buffer.line_to_byte(line + 1);
        if let Some((revision, range)) = &self.spans_for
            && *revision == self.buffer.revision()
            && range.start <= start
            && end <= range.end
        {
            // Positions stay in the whole text's bytes, as the other spans are.
            let first = self.spans.partition_point(|(r, _)| r.end <= start);
            return self.spans[first..].iter().take_while(|(r, _)| r.start < end).cloned().collect();
        }
        let Some(highlighter) = &mut self.highlighter else { return Vec::new() };
        highlighter.sync(&self.buffer);
        highlighter.spans(self.buffer.rope(), start..end)
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
        self.single_cursor();
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
        // Every match, not just the ones highlighted (which stop at 10,000).
        let edits: Vec<(Range<usize>, String)> = search
            .query
            .find_every(regex, &text)
            .into_iter()
            .map(|bytes| {
                let chars = rope.byte_to_char(bytes.start)..rope.byte_to_char(bytes.end);
                (chars, search.query.replacement_for(regex, &text, bytes, replacement))
            })
            .collect();
        self.record_undo(EditKind::Other);
        for (range, new_text) in edits.iter().rev() {
            self.buffer.replace(range.clone(), new_text);
        }
        self.single_cursor();
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
        self.single_cursor();
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
        } else if self.signature.card.is_some() {
            self.close_signature(cx);
        } else if !self.extra.is_empty() {
            self.single_cursor();
            cx.notify();
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
        self.check_ghost();
        if self.hover_from_keyboard {
            self.close_hover(cx);
        }
        self.signature_after_move(cx);
        self.unfold_around_caret(cx);
        self.refresh_symbol_marks(cx);
        self.refresh_blame(cx);
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

    /// One character to the left, where a character is what people see as one: a flag,
    /// an emoji with its skin tone, a letter with its accent.
    fn left_of(&self, offset: usize) -> usize {
        let (line, col) = self.buffer.point(offset);
        match (line, col) {
            (0, 0) => 0,
            (_, 0) => self.buffer.offset(line - 1, usize::MAX),
            _ => {
                let text = self.buffer.line_text(line);
                let start = grapheme_columns(&text).into_iter().take_while(|&c| c < col).last().unwrap_or(col - 1);
                offset - (col - start)
            }
        }
    }

    fn right_of(&self, offset: usize) -> usize {
        let (line, col) = self.buffer.point(offset);
        if col >= self.buffer.line_len(line) {
            if line + 1 < self.buffer.len_lines() { self.buffer.offset(line + 1, 0) } else { offset }
        } else {
            let text = self.buffer.line_text(line);
            let next = grapheme_columns(&text).into_iter().find(|&c| c > col).unwrap_or(col + 1);
            offset + (next - col)
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

    /// Up and down move by rows on screen, so a wrapped line takes several presses.
    /// The goal column is a screen column, kept while moving through shorter rows.
    fn move_vertically(&mut self, lines: isize, select: bool, cx: &mut Context<Self>) {
        self.close_completion(cx);
        if !select && !self.selection.is_empty() && lines.abs() == 1 {
            let edge = if lines < 0 { self.selection.range().start } else { self.selection.range().end };
            self.selection = Selection::caret(edge);
        }
        self.wrap.update(&self.buffer, self.wrap.width(), &self.block_specs());
        let (line, col) = self.buffer.point(self.selection.head);
        let (row, display_col) = self.wrap.to_display(line, col, &self.buffer);
        let goal = self.goal_column.unwrap_or(display_col);
        let last = self.wrap.rows() - 1;
        let mut target = row as isize + lines;
        // Step over rows that aren't text (the AI's field, a note...).
        let step = lines.signum();
        while target >= 0 && (target as usize) <= last && self.wrap.block_at(target as usize).is_some() {
            target += step;
        }
        let offset = if target < 0 {
            0
        } else if target as usize > last {
            self.buffer.len_chars()
        } else {
            self.wrap.to_offset(target as usize, goal, &self.buffer)
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
        self.for_each_cursor(cx, |this, cx| {
            let target = if this.selection.is_empty() {
                this.left_of(this.selection.head)
            } else {
                this.selection.range().start
            };
            this.move_head(target, false, cx);
        });
    }

    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            let target =
                if this.selection.is_empty() { this.right_of(this.selection.head) } else { this.selection.range().end };
            this.move_head(target, false, cx);
        });
    }

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_vertically(-1, false, cx);
        });
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_vertically(1, false, cx);
        });
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(this.left_of(this.selection.head), true, cx);
        });
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(this.right_of(this.selection.head), true, cx);
        });
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_vertically(-1, true, cx);
        });
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_vertically(1, true, cx);
        });
    }

    fn move_word_left(&mut self, _: &MoveWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(this.word_left_of(this.selection.head), false, cx);
        });
    }

    fn move_word_right(&mut self, _: &MoveWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(this.word_right_of(this.selection.head), false, cx);
        });
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(this.word_left_of(this.selection.head), true, cx);
        });
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(this.word_right_of(this.selection.head), true, cx);
        });
    }

    fn move_line_start(&mut self, _: &MoveLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(this.line_start_smart(this.selection.head), false, cx);
        });
    }

    fn move_line_end(&mut self, _: &MoveLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            let (line, _) = this.caret_point();
            this.move_head(this.buffer.offset(line, usize::MAX), false, cx);
        });
    }

    fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(this.line_start_smart(this.selection.head), true, cx);
        });
    }

    fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            let (line, _) = this.caret_point();
            this.move_head(this.buffer.offset(line, usize::MAX), true, cx);
        });
    }

    fn move_doc_start(&mut self, _: &MoveDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(0, false, cx);
        });
    }

    fn move_doc_end(&mut self, _: &MoveDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(this.buffer.len_chars(), false, cx);
        });
    }

    fn select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(0, true, cx);
        });
    }

    fn select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.move_head(this.buffer.len_chars(), true, cx);
        });
    }

    /// Page Up and Down move the view by a page too, with the caret, not just the caret.
    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.page_lines();
        self.scroll.target_y = (self.scroll.target_y - page as f32 * f32::from(self.line_height())).max(0.);
        self.for_each_cursor(cx, |this, cx| {
            this.move_vertically(-page, false, cx);
        });
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.page_lines();
        self.scroll.target_y += page as f32 * f32::from(self.line_height());
        self.for_each_cursor(cx, |this, cx| {
            this.move_vertically(page, false, cx);
        });
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.single_cursor();
        self.selection = Selection { anchor: 0, head: self.buffer.len_chars() };
        self.last_activity = Instant::now();
        cx.notify();
    }

    // ---------- editing ----------

    fn record_undo(&mut self, kind: EditKind) {
        // A command run on every cursor is a single undo step.
        if self.batch.as_ref().is_some_and(|b| b.recorded) {
            return;
        }
        let now = Instant::now();
        let grouped = matches!(self.last_edit, Some((last, at, _)) if last == kind && kind != EditKind::Other && now - at < UNDO_GROUP);
        if !grouped {
            let (selection, extra) = match &mut self.batch {
                Some(batch) => (batch.before[0], batch.before[1..].to_vec()),
                None => (self.selection, self.extra.iter().map(|c| c.selection).collect()),
            };
            self.undo_stack.push(Snapshot {
                text: self.buffer.rope().clone(),
                version: self.buffer.version(),
                selection,
                extra,
            });
            if self.undo_stack.len() > 1000 {
                self.undo_stack.remove(0);
            }
        }
        self.redo_stack.clear();
        self.last_edit = Some((kind, now, self.selection.head));
        if let Some(batch) = &mut self.batch {
            batch.recorded = true;
        }
    }

    fn edit(&mut self, range: Range<usize>, text: &str, kind: EditKind, cx: &mut Context<Self>) {
        if kind == EditKind::Other {
            self.close_completion(cx);
        }
        if self.starts_undo_step(&range, text, kind) {
            self.last_edit = None;
        }
        self.record_undo(kind);
        let end = self.buffer.replace(range, text);
        if let Some((_, _, caret)) = &mut self.last_edit
            && self.batch.is_none()
        {
            *caret = end;
        }
        self.selection = Selection::caret(end);
        self.marked = None;
        self.goal_column = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    /// Whether this edit starts a new undo step even right after another of its kind:
    /// undo takes back a word at a time, and never more than one place's typing.
    fn starts_undo_step(&self, range: &Range<usize>, text: &str, kind: EditKind) -> bool {
        let Some((last, _, caret)) = self.last_edit else { return false };
        if last != kind || kind == EditKind::Other {
            return false;
        }
        // Typing somewhere else (with several cursors, every cursor moves: skip that).
        let elsewhere = self.batch.is_none() && range.start != caret && range.end != caret;
        // A new line, or a new word after a space.
        let new_line = text.contains('\n');
        let new_word = kind == EditKind::Typing
            && text.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_')
            && range.start > 0
            && self.buffer.char_at(range.start - 1).is_some_and(char::is_whitespace);
        elsewhere || new_line || new_word
    }

    fn delete_or(&mut self, range: impl FnOnce(&Self) -> Range<usize>, cx: &mut Context<Self>) {
        let range = if self.selection.is_empty() { range(self) } else { self.selection.range() };
        if !range.is_empty() {
            self.edit(range, "", EditKind::Deleting, cx);
            self.completion_after_delete(cx);
        }
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            if this.empty_pair_around_caret() {
                let head = this.selection.head;
                return this.edit(head - 1..head + 1, "", EditKind::Deleting, cx);
            }
            this.delete_or(
                |this| {
                    let head = this.selection.head;
                    let (line, col) = this.buffer.point(head);
                    let before = this.buffer.line_text(line).chars().take(col).collect::<String>();
                    let width = this.style.indent.width();
                    if col > 0 && before.chars().all(|c| c == ' ') && this.style.indent != IndentStyle::Tabs {
                        let stop = (col - 1) / width * width;
                        return head - (col - stop)..head;
                    }
                    this.left_of(head)..head
                },
                cx,
            );
        });
    }

    fn backspace_word(&mut self, _: &BackspaceWord, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.delete_or(|this| this.word_left_of(this.selection.head)..this.selection.head, cx);
        });
    }

    fn backspace_line(&mut self, _: &BackspaceLine, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.delete_or(
                |this| {
                    let (line, _) = this.caret_point();
                    this.buffer.line_to_char(line)..this.selection.head
                },
                cx,
            );
        });
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.delete_or(|this| this.selection.head..this.right_of(this.selection.head), cx);
        });
    }

    fn delete_word(&mut self, _: &DeleteWord, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.delete_or(|this| this.selection.head..this.word_right_of(this.selection.head), cx);
        });
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            let mut range = this.selection.range();
            let (line, col) = this.buffer.point(range.start);
            let line_text = this.buffer.line_text(line);
            // The indentation up to the caret only: Enter inside it doesn't double it.
            let indent: String = line_text.chars().take(col).take_while(|c| *c == ' ' || *c == '\t').collect();
            // On a line of only spaces, those spaces don't stay behind.
            if line_text.chars().take(col).all(|c| c == ' ' || c == '\t') && col > 0 {
                range.start = this.buffer.line_to_char(line);
            }
            let before = range.start.checked_sub(1).and_then(|i| this.buffer.char_at(i));
            let after = this.buffer.char_at(range.end);
            // Python blocks open with a colon.
            let python_block = before == Some(':') && this.language_name() == "Python";
            let opens = matches!(before, Some('{' | '(' | '[')) || python_block;
            let inner = format!("{indent}{}", if opens { this.style.indent.unit() } else { String::new() });
            // The file's own line break: "\r\n" in a Windows file.
            let nl = this.style.line_ending.text();
            if opens
                && matches!((before, after), (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']')))
            {
                let text = format!("{nl}{inner}{nl}{indent}");
                let caret = range.start + nl.chars().count() + inner.chars().count();
                this.edit(range, &text, EditKind::Other, cx);
                this.selection = Selection::caret(caret);
            } else {
                this.edit(range, &format!("{nl}{inner}"), EditKind::Other, cx);
            }
        });
        // A new line is a good moment for a suggestion (the body after `def f():`...).
        self.schedule_ghost(None, cx);
    }

    fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        self.tab_key(cx);
    }

    pub(crate) fn tab_key(&mut self, cx: &mut Context<Self>) {
        if self.multi_cursor() && self.batch.is_none() {
            if self.all_selections().iter().any(|s| !s.is_empty()) {
                self.merge_cursors_on_shared_lines();
            }
            return self.for_each_cursor(cx, |this, cx| this.tab_key(cx));
        }
        // With anything selected, Tab indents the selected lines (Shift+Tab outdents).
        // This is the easy way on every keyboard layout; Cmd+] / Cmd+[ need [ and ],
        // which on many non-US Mac keyboards take several keys already.
        if !self.selection.is_empty() {
            return self.indent_lines(cx);
        }
        let range = self.selection.range();
        let text = match self.style.indent {
            IndentStyle::Tabs => "\t".to_string(),
            // Spaces up to the next indentation stop.
            IndentStyle::Spaces(n) => " ".repeat(n - self.buffer.point(range.start).1 % n),
        };
        self.edit(range, &text, EditKind::Typing, cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selections(cx);
    }

    /// With nothing selected, Cut takes the whole line, as Copy copies it.
    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let whole_lines = self.all_selections().iter().all(|s| s.is_empty());
        if !self.copy_selections(cx) {
            return;
        }
        if whole_lines {
            self.merge_cursors_on_shared_lines();
            return self.for_each_cursor(cx, |this, cx| this.delete_lines(cx));
        }
        self.for_each_cursor(cx, |this, cx| {
            if !this.selection.is_empty() {
                this.edit(this.selection.range(), "", EditKind::Other, cx);
            }
        });
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else { return };
        let Some(text) = item.text() else { return };
        let kind = item.metadata().cloned().unwrap_or_default();
        // Pasted line breaks become the file's own.
        let text = text.replace("\r\n", "\n");
        let text = match self.style.line_ending {
            crate::file_style::LineEnding::Crlf => text.replace('\n', "\r\n"),
            crate::file_style::LineEnding::Lf => text,
        };
        self.paste_text(text, &kind, cx);
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
        let extra = self.extra.iter().map(|c| c.selection).collect();
        to.push(Snapshot {
            text: self.buffer.rope().clone(),
            version: self.buffer.version(),
            selection: self.selection,
            extra,
        });
        self.buffer.restore_version(snapshot.text, snapshot.version);
        self.selection = snapshot.selection;
        self.extra = snapshot.extra.into_iter().map(|selection| Cursor { selection, goal: None }).collect();
        self.last_edit = None;
        self.marked = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save_from_keyboard(cx);
    }

    /// Writes the buffer to its file. Returns false if there's no file or writing failed.
    /// Changes made as one undo step, the caret staying on its line and column: (char
    /// range, new text), in any order, not overlapping.
    pub(crate) fn apply_char_edits(&mut self, mut edits: Vec<(Range<usize>, String)>, cx: &mut Context<Self>) {
        if edits.is_empty() {
            return;
        }
        let (line, col) = self.caret_point();
        self.record_undo(EditKind::Other);
        edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
        for (range, text) in edits {
            self.buffer.replace(range, &text);
        }
        self.single_cursor();
        self.selection = Selection::caret(self.buffer.offset(line, col));
        self.goal_column = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        cx.notify();
    }

    /// What saving tidies, as the file's .editorconfig asks: spaces at line ends, the
    /// final line break, and stray "\n" breaks in a "\r\n" file.
    fn tidy_for_save(&mut self, cx: &mut Context<Self>) {
        let style = self.style.clone();
        let text = self.buffer.to_string();
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut trailing: Option<usize> = None; // where a run of spaces at the end of a line starts
        let mut previous = None;
        let mut count = 0;
        for (i, c) in text.chars().enumerate() {
            match c {
                ' ' | '\t' => {
                    trailing.get_or_insert(i);
                }
                '\n' | '\r' => {
                    if let Some(start) = trailing.take().filter(|_| style.trim_trailing) {
                        edits.push((start..i, String::new()));
                    }
                    if c == '\n' && previous != Some('\r') && style.line_ending == crate::file_style::LineEnding::Crlf {
                        edits.push((i..i, "\r".into()));
                    }
                }
                _ => trailing = None,
            }
            previous = Some(c);
            count = i + 1;
        }
        if let Some(start) = trailing.filter(|_| style.trim_trailing) {
            edits.push((start..count, String::new()));
        }
        if style.final_newline == Some(true) && !text.is_empty() && !text.ends_with('\n') {
            edits.push((count..count, style.line_ending.text().into()));
        }
        self.apply_char_edits(edits, cx);
    }

    /// Opens a second copy of `source` (the same file on the other side): its text (unsaved
    /// edits included), its caret, its style. The language server is left to the first.
    pub fn twin(source: TwinSource, lsp: Option<Entity<LspStore>>, cx: &mut Context<Self>) -> Self {
        let mut buffer = Buffer::from_text(&source.text);
        if source.dirty {
            buffer.mark_unsaved();
        }
        let mut editor = Self::new(buffer, Some(source.path), cx);
        editor.style = source.style;
        editor.lsp_follower = true;
        if let Some(lsp) = lsp {
            editor.attach_lsp(lsp, cx);
        }
        editor.reload_git_base(cx);
        let (line, column, top) = source.view;
        editor.restore_view(line, column, top, cx);
        editor
    }

    /// What a second copy of this editor's file starts from.
    pub fn twin_source(&self) -> Option<TwinSource> {
        Some(TwinSource {
            text: self.buffer.to_string(),
            dirty: self.buffer.is_dirty(),
            path: self.path.clone()?,
            style: self.style.clone(),
            view: self.view_state(),
        })
    }

    /// Replays the other copy's edits here, keeping this side's carets where they were in
    /// the text. Without the edits (after an undo there), takes its whole text.
    pub fn apply_twin_edits(
        &mut self,
        edits: Option<Vec<crate::buffer::Edit>>,
        text: &Rope,
        saved: bool,
        cx: &mut Context<Self>,
    ) {
        self.record_undo(EditKind::Typing);
        match edits {
            Some(edits) => {
                for edit in edits {
                    let rope = self.buffer.rope();
                    let start = rope.byte_to_char(edit.start_byte.min(rope.len_bytes()));
                    let end = rope.byte_to_char(edit.old_end_byte.min(rope.len_bytes()));
                    let added = edit.text.chars().count();
                    let shift = |o: usize| {
                        if o <= start {
                            o
                        } else if o >= end {
                            o + added - (end - start)
                        } else {
                            start + added
                        }
                    };
                    self.selection =
                        Selection { anchor: shift(self.selection.anchor), head: shift(self.selection.head) };
                    for cursor in &mut self.extra {
                        cursor.selection =
                            Selection { anchor: shift(cursor.selection.anchor), head: shift(cursor.selection.head) };
                    }
                    self.buffer.replace(start..end, &edit.text);
                }
            }
            None => {
                let (line, column) = self.caret_point();
                self.buffer.replace(0..self.buffer.len_chars(), &text.to_string());
                self.single_cursor();
                self.selection = Selection::caret(self.buffer.offset(line, column));
            }
        }
        if saved {
            self.buffer.mark_saved();
        }
        self.text_changed(cx);
        cx.notify();
    }

    /// Types `text` at `at`, as the keyboard would (for tests elsewhere in the crate).
    #[cfg(test)]
    pub fn type_text_for_test(&mut self, at: usize, text: &str, cx: &mut Context<Self>) {
        self.edit(at..at, text, EditKind::Typing, cx);
    }

    /// Switches the file's line endings, converting every line break.
    pub fn set_line_ending(&mut self, ending: crate::file_style::LineEnding, cx: &mut Context<Self>) {
        self.style.line_ending = ending;
        let mut edits = Vec::new();
        let mut previous = None;
        for (i, c) in self.buffer.rope().chars().enumerate() {
            match (ending, c, previous) {
                (crate::file_style::LineEnding::Crlf, '\n', p) if p != Some('\r') => edits.push((i..i, "\r".into())),
                (crate::file_style::LineEnding::Lf, '\n', Some('\r')) => edits.push((i - 1..i, String::new())),
                _ => {}
            }
            previous = Some(c);
        }
        self.apply_char_edits(edits, cx);
        cx.notify();
    }

    /// Indents with this from now on (the lines already there stay as they are).
    pub fn set_indent(&mut self, indent: IndentStyle, cx: &mut Context<Self>) {
        self.style.indent = indent;
        cx.notify();
    }

    pub fn save_to_disk(&mut self, cx: &mut Context<Self>) -> bool {
        if self.path.is_none() {
            cx.emit(EditorEvent::NeedsPath);
            return false;
        }
        self.tidy_for_save(cx);
        let Some(path) = &self.path else { return false };
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

    /// The list of suggestions from the language server, kept quiet: names line up
    /// under the word being typed, the typed letters stand out, a dot gives the kind, and
    /// only the selected row shows its details.
    fn render_completions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        const ROW: f32 = ui::ROW_SM;
        const ROWS: usize = 6;
        const PAD: f32 = 4.;
        const DOT_COLUMN: f32 = 18.;
        let menu = self.completion.as_ref()?;
        let bounds = self.caret_bounds(menu.word_start().min(self.selection.head))?;
        let theme = cx.global::<Theme>();
        let code_font = cx.global::<Fonts>().code.clone();
        let rows = (0..menu.shown.len()).map(|ix| {
            let suggestion = menu.suggestion(ix);
            let selected = ix == menu.selected;
            let typed = HighlightStyle { color: Some(theme.foreground), ..Default::default() };
            let label =
                StyledText::new(suggestion.label.clone()).with_highlights(menu.shown[ix].1.iter().filter_map(|&b| {
                    let len = suggestion.label.get(b..)?.chars().next()?.len_utf8();
                    Some((b..b + len, typed))
                }));
            div()
                .id(ix)
                .h(px(ROW))
                .flex()
                .items_center()
                .pr(px(10.))
                .rounded(px(ui::R_ROW))
                .cursor_pointer()
                .when(selected, |row| row.bg(theme.accent_soft))
                .when(!selected, |row| row.hover(|r| r.bg(theme.hairline)))
                .child(
                    div()
                        .w(px(DOT_COLUMN))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .child(div().size(px(4.)).rounded_full().bg(kind_color(suggestion.kind, theme))),
                )
                .child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_color(if selected { theme.foreground } else { theme.muted })
                        .child(label),
                )
                .when(selected, |row| {
                    row.children(suggestion.detail.clone().map(|detail| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .pl(px(16.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_right()
                            .text_size(px(ui::T_SM))
                            .text_color(theme.muted)
                            .child(detail)
                    }))
                })
                .active(|s| s.opacity(0.7))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.accept_completion(ix, cx)))
        });
        let list = div()
            .id("completions")
            .occlude()
            .track_scroll(&menu.scroll)
            .overflow_y_scroll()
            .min_w(px(220.))
            .max_w(px(480.))
            .max_h(px(ROW * ROWS as f32 + PAD * 2.))
            .p(px(PAD))
            .rounded(px(ui::R_POPOVER))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.hairline)
            .shadow_md()
            .font_family(code_font)
            .text_size(self.font_size * 0.93)
            .children(rows);
        // The names start exactly where the word being typed does.
        Some(
            deferred(
                anchored()
                    .position(point(bounds.left() - px(PAD + DOT_COLUMN), bounds.bottom() + px(3.)))
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
                    .rounded(px(ui::R_ROW))
                    .bg(theme.sunken)
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
            .rounded(px(ui::R_POPOVER))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.hairline)
            .shadow_lg()
            .text_size(px(ui::T_MD))
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
        let row = y.floor() as usize;
        if row >= self.wrap.rows() {
            return self.buffer.len_chars();
        }
        let x = position.x - layout.text_origin.x;
        match row.checked_sub(layout.first_row).and_then(|i| layout.rows.get(i)) {
            Some(r) => {
                let byte = r.text_byte(r.shaped.closest_index_for_x(x - r.x));
                // The row can show more than its text (a ghost completion): clicks past it land at its end.
                let chars = r.text.get(..byte).map_or(r.text.chars().count(), |t| t.chars().count());
                let col = r.row.cols.start + chars;
                // Past the end of a wrapped row, stay on it rather than jump to the next.
                let col =
                    if r.row.last { col } else { col.min(r.row.cols.end.saturating_sub(1)).max(r.row.cols.start) };
                self.buffer.offset(r.row.line, col)
            }
            None => self.wrap.to_offset(row, (x / layout.char_width).round().max(0.) as usize, &self.buffer),
        }
    }

    /// The column under a window position on its line, counting past the line's end as
    /// if it went on in spaces (a box selection can reach beyond short lines).
    fn column_at_x(&self, position: Point<Pixels>) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let offset = self.offset_at(position);
        let (line, col) = self.buffer.point(offset);
        let len = self.buffer.line_len(line);
        if col < len {
            return Some(col);
        }
        // Past the end: the line's width on screen, then whole columns beyond it.
        let text = self.buffer.line_text(line);
        let width = text.chars().fold(0, |c, ch| c + crate::wrap::char_columns(ch, c));
        let x = f32::from(position.x - layout.text_origin.x) / f32::from(layout.char_width);
        let screen_col = x.round().max(0.) as usize;
        Some(len + screen_col.saturating_sub(width))
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
        if let Some(line) = self.breakpoint_click(event.position) {
            return self.toggle_breakpoint_at(line, cx);
        }
        if let Some(line) = self.fold_click(event.position) {
            return self.toggle_fold(line, cx);
        }
        // A line pinned at the top: go to it.
        if let Some(&(_, line)) =
            self.layout.as_ref().and_then(|l| l.sticky.iter().find(|(b, _)| b.contains(&event.position)))
        {
            cx.emit(EditorEvent::Jumped { from: self.caret_point() });
            let indent = self.buffer.line_text(line).chars().take_while(|c| c.is_whitespace()).count();
            return self.set_caret_point((line, indent), cx);
        }
        let offset = self.offset_at(event.position);
        // ⌥⇧-drag selects a box: the same columns on every line it crosses.
        if event.modifiers.alt && event.modifiers.shift && event.click_count == 1 {
            self.remember_cursors();
            let line = self.buffer.point(offset).0;
            let column = self.column_at_x(event.position).unwrap_or_else(|| self.buffer.point(offset).1);
            self.hover_suppressed = true;
            self.single_cursor();
            self.selection = Selection::caret(offset);
            self.dragging = Some(DragUnit::Column(line, column));
            return self.touch(cx);
        }
        // Cmd+Shift+click (Ctrl+Shift elsewhere) adds a cursor, or removes the one clicked.
        // Not Alt+click: holding Alt opens the info card.
        let add_cursor = event.modifiers.secondary() && event.modifiers.shift;
        let unit = match event.click_count {
            2 => DragUnit::Word(self.word_at(offset)),
            3 => DragUnit::Line(self.line_range(offset)),
            _ => DragUnit::Char,
        };
        match &unit {
            DragUnit::Word(range) | DragUnit::Line(range) => {
                self.selection = Selection { anchor: range.start, head: range.end }
            }
            DragUnit::Char if add_cursor => {
                self.remember_cursors();
                self.toggle_cursor_at(offset)
            }
            DragUnit::Char if event.modifiers.shift => self.selection.head = offset,
            DragUnit::Char => self.selection = Selection::caret(offset),
            // Started above, before this match.
            DragUnit::Column(..) => {}
        }
        if !(add_cursor || event.modifiers.shift) {
            self.single_cursor();
        }
        self.goal_column = None;
        self.dragging = Some(unit);
        self.touch(cx);
        self.reveal_only = true;
        if event.modifiers.secondary() && !add_cursor && event.click_count == 1 {
            self.dragging = None;
            self.go_to_definition_at(offset, cx);
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let was_over_scrollbar = self.over_scrollbar();
        let was_over_gutter = self.over_gutter();
        self.mouse_position = Some(event.position);
        // The fold chevrons show while the mouse is over the gutter.
        if was_over_gutter != self.over_gutter() {
            cx.notify();
        }
        if self.scrollbar_drag.is_some() && event.pressed_button == Some(MouseButton::Left) {
            return self.scrollbar_drag_to(event.position, cx);
        }
        if was_over_scrollbar != self.over_scrollbar() {
            cx.notify();
        }
        if self.alt_held || self.secondary_held || self.link_word.is_some() {
            self.update_hover(cx);
        }
        // Dragging is followed window-wide, from the element (see `drag_to`).
    }

    fn over_gutter(&self) -> bool {
        match (&self.layout, self.mouse_position) {
            (Some(layout), Some(p)) => layout.bounds.contains(&p) && p.x < layout.text_bounds.left(),
            _ => false,
        }
    }

    /// The line whose fold a click toggles: its chevron in the gutter, or the "⋯" after a
    /// folded line.
    fn fold_click(&mut self, position: Point<Pixels>) -> Option<usize> {
        let (line, first_row, last_row, in_gutter, past_text) = {
            let layout = self.layout.as_ref()?;
            let row = ((position.y - layout.text_origin.y) / layout.line_height).floor();
            let r = layout.rows.get((row as usize).checked_sub(layout.first_row)?)?;
            if r.row.block.is_some() || !layout.bounds.contains(&position) {
                return None;
            }
            let text_end = r.shaped.x_for_index(r.shown_byte(r.text.len()));
            (
                r.row.line,
                r.row.cols.start == 0,
                r.row.last,
                position.x < layout.text_bounds.left(),
                position.x - layout.text_origin.x - r.x > text_end,
            )
        };
        if in_gutter {
            let can_fold = self.is_folded(line) || self.foldable().iter().any(|f| f.start == line);
            return (can_fold && first_row).then_some(line);
        }
        (last_row && self.is_folded(line) && past_text).then_some(line)
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some() || self.scrollbar_drag.is_some()
    }

    /// The mouse moved while a selection or the scrollbar is being dragged, wherever it is
    /// in the window. Past the top or bottom, the text scrolls, faster the further away.
    pub fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.mouse_position = Some(position);
        if self.scrollbar_drag.is_some() {
            return self.scrollbar_drag_to(position, cx);
        }
        if self.dragging.is_none() {
            return;
        }
        if let Some(layout) = &self.layout {
            let (top, bottom) = (layout.text_bounds.top(), layout.text_bounds.bottom());
            let past = if position.y < top {
                position.y - top
            } else if position.y > bottom {
                position.y - bottom
            } else {
                px(0.)
            };
            if past != px(0.) {
                let step = f32::from(past) * 0.25;
                self.scroll.target_y = (self.scroll.target_y + step).max(0.);
                self.scroll.y = self.scroll.target_y;
            }
        }
        let offset = self.offset_at(position);
        let (origin, target) = match &self.dragging {
            None => return,
            Some(DragUnit::Char) => {
                self.selection.head = offset;
                self.touch(cx);
                self.reveal_only = true;
                return;
            }
            &Some(DragUnit::Column(line, column)) => {
                let to_line = self.buffer.point(offset).0;
                let to_column = self.column_at_x(position).unwrap_or_else(|| self.buffer.point(offset).1);
                self.select_box((line, column), (to_line, to_column));
                self.touch(cx);
                self.reveal_only = true;
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
        self.reveal_only = true;
    }

    /// While dragging past an edge without moving, keeps scrolling.
    pub fn continue_drag(&mut self, cx: &mut Context<Self>) -> bool {
        let (Some(position), Some(layout)) = (self.mouse_position, &self.layout) else { return false };
        let outside = position.y < layout.text_bounds.top() || position.y > layout.text_bounds.bottom();
        if self.dragging.is_some() && outside {
            self.drag_to(position, cx);
            return true;
        }
        false
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
        // With Shift too it's Cmd+Shift+click (add a cursor), so no link underline.
        self.secondary_held = event.modifiers.secondary() && !event.modifiers.shift;
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

    /// Shows what's known about the code at the caret: its problems, then its type and docs.
    pub fn show_info_now(&mut self, cx: &mut Context<Self>) {
        self.show_info_at_caret(cx);
    }

    fn show_info(&mut self, _: &ShowInfo, _: &mut Window, cx: &mut Context<Self>) {
        self.show_info_at_caret(cx);
    }

    fn show_completions(&mut self, _: &ShowCompletions, _: &mut Window, cx: &mut Context<Self>) {
        self.show_completions_now(cx);
    }

    // The suggestion list's keys also drive ⌘.'s list of fixes while it's open.

    fn completion_next(&mut self, _: &CompletionNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.fix_menu.is_some() {
            return self.move_fix(1, cx);
        }
        self.move_completion(1, cx);
    }

    fn completion_previous(&mut self, _: &CompletionPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if self.fix_menu.is_some() {
            return self.move_fix(-1, cx);
        }
        self.move_completion(-1, cx);
    }

    fn confirm_completion(&mut self, _: &ConfirmCompletion, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &self.fix_menu {
            return self.accept_fix(menu.selected, cx);
        }
        let selected = self.completion.as_ref().map_or(0, |m| m.selected);
        self.accept_completion(selected, cx);
    }

    /// Runs a line command on each cursor's lines, once per line.
    fn on_each_cursors_lines(&mut self, cx: &mut Context<Self>, f: impl FnMut(&mut Self, &mut Context<Self>)) {
        self.merge_cursors_on_shared_lines();
        self.for_each_cursor(cx, f);
    }

    fn toggle_comment_action(&mut self, _: &ToggleComment, _: &mut Window, cx: &mut Context<Self>) {
        self.on_each_cursors_lines(cx, |this, cx| this.toggle_comment(cx));
    }

    fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        self.on_each_cursors_lines(cx, |this, cx| this.indent_lines(cx));
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.on_each_cursors_lines(cx, |this, cx| this.outdent_lines(cx));
    }

    // Moving lines works on the main cursor only: with several, the blocks would trip over each other.
    fn move_line_up(&mut self, _: &MoveLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.single_cursor();
        self.move_lines(false, cx);
    }

    fn move_line_down(&mut self, _: &MoveLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.single_cursor();
        self.move_lines(true, cx);
    }

    fn duplicate_line_up(&mut self, _: &DuplicateLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.on_each_cursors_lines(cx, |this, cx| this.duplicate_lines(false, cx));
    }

    fn duplicate_line_down(&mut self, _: &DuplicateLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.on_each_cursors_lines(cx, |this, cx| this.duplicate_lines(true, cx));
    }

    fn delete_line(&mut self, _: &DeleteLine, _: &mut Window, cx: &mut Context<Self>) {
        self.on_each_cursors_lines(cx, |this, cx| this.delete_lines(cx));
    }

    fn select_line_action(&mut self, _: &SelectLine, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| this.select_line(cx));
    }

    fn add_next_occurrence_action(&mut self, _: &AddNextOccurrence, _: &mut Window, cx: &mut Context<Self>) {
        self.remember_cursors();
        self.add_next_occurrence(cx);
    }

    fn add_cursors_to_line_ends(&mut self, _: &AddCursorsToLineEnds, _: &mut Window, cx: &mut Context<Self>) {
        self.remember_cursors();
        self.cursors_at_line_ends(cx);
    }

    fn undo_cursor(&mut self, _: &UndoCursor, _: &mut Window, cx: &mut Context<Self>) {
        self.restore_cursors(cx);
    }

    fn select_all_occurrences_action(&mut self, _: &SelectAllOccurrences, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_occurrences(cx);
    }

    fn add_cursor_above(&mut self, _: &AddCursorAbove, _: &mut Window, cx: &mut Context<Self>) {
        self.remember_cursors();
        self.add_cursor_vertically(false, cx);
    }

    fn add_cursor_below(&mut self, _: &AddCursorBelow, _: &mut Window, cx: &mut Context<Self>) {
        self.remember_cursors();
        self.add_cursor_vertically(true, cx);
    }

    fn inline_assist(&mut self, _: &InlineAssist, window: &mut Window, cx: &mut Context<Self>) {
        self.open_inline_assist(false, window, cx);
    }

    /// Where the caret is and the first line shown, for the session.
    pub fn view_state(&self) -> (usize, usize, usize) {
        let (line, column) = self.caret_point();
        let top_row = (self.scroll.y / f32::from(self.line_height())).round().max(0.) as usize;
        (line, column, self.wrap.line_of_row(top_row))
    }

    /// Puts the caret and the view back as they were, without scrolling there visibly.
    pub fn restore_view(&mut self, line: usize, column: usize, top_line: usize, cx: &mut Context<Self>) {
        self.single_cursor();
        self.selection = Selection::caret(self.buffer.offset(line, column));
        self.goal_column = None;
        // Before its first frame, the editor's rows aren't laid out yet.
        self.wrap.update(&self.buffer, self.wrap.width(), &self.block_specs());
        let top = self.wrap.first_row(top_line.min(self.buffer.len_lines().saturating_sub(1))) as f32;
        self.scroll.y = top * f32::from(self.line_height());
        self.scroll.target_y = self.scroll.y;
        self.caret.placed = false;
        cx.notify();
    }

    /// Opens the ⌘I field to ask about the code at the caret.
    pub fn ask_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_inline_assist(true, window, cx);
    }

    fn cancel_completion(&mut self, _: &CancelCompletion, _: &mut Window, cx: &mut Context<Self>) {
        self.close_completion(cx);
        self.close_fixes(cx);
    }

    fn go_to_definition(&mut self, _: &GoToDefinition, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_definition_at(self.selection.head, cx);
    }

    fn go_to_type_definition(&mut self, _: &GoToTypeDefinition, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_target(crate::lsp_store::Target::TypeDefinition, cx);
    }

    fn go_to_implementation(&mut self, _: &GoToImplementation, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_target(crate::lsp_store::Target::Implementation, cx);
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let line_height = f32::from(self.line_height());
        match event.delta {
            // Trackpads report exact pixels and already feel smooth: follow them 1:1. A swipe
            // keeps to the direction it started in, so scrolling down doesn't drift sideways.
            ScrollDelta::Pixels(delta) => {
                if matches!(event.touch_phase, gpui::TouchPhase::Started) {
                    self.scroll.swipe_sideways = None;
                }
                let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                if self.scroll.swipe_sideways.is_none() && (dx.abs() > 1. || dy.abs() > 1.) {
                    self.scroll.swipe_sideways = Some(dx.abs() > dy.abs());
                }
                match self.scroll.swipe_sideways {
                    Some(true) => self.scroll.x -= dx,
                    Some(false) => {
                        self.scroll.y -= dy;
                        self.scroll.target_y = self.scroll.y;
                    }
                    None => {}
                }
                if matches!(event.touch_phase, gpui::TouchPhase::Ended) {
                    self.scroll.swipe_sideways = None;
                }
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
        let (row, x) = crate::element::position(
            &layout.rows,
            layout.first_row,
            &self.wrap,
            &self.buffer,
            layout.char_width,
            line,
            col,
        );
        Some(Bounds::new(
            point(layout.text_origin.x + x, layout.text_origin.y + layout.line_height * row as f32),
            size(px(2.), layout.line_height),
        ))
    }
}

/// What a mouse drag selects by, set by the click that started it.
enum DragUnit {
    Char,
    /// ⌥⇧-drag: a box from this (line, column), a cursor on each line.
    Column(usize, usize),
    Word(Range<usize>),
    Line(Range<usize>),
}

/// A one-letter hint of what a suggestion is, in the color code uses for it.
/// The dot beside a suggestion: the color its kind has in code.
fn kind_color(kind: Option<lsp_types::CompletionItemKind>, theme: &Theme) -> gpui::Hsla {
    use crate::theme::Syntax;
    use lsp_types::CompletionItemKind as K;
    match kind {
        Some(K::FUNCTION | K::METHOD | K::CONSTRUCTOR) => theme.syntax(Syntax::Function),
        Some(K::STRUCT | K::CLASS | K::TYPE_PARAMETER | K::ENUM | K::ENUM_MEMBER | K::INTERFACE) => {
            theme.syntax(Syntax::Type)
        }
        Some(K::FIELD | K::PROPERTY) => theme.syntax(Syntax::Property),
        Some(K::CONSTANT) => theme.syntax(Syntax::Number),
        Some(K::MODULE | K::KEYWORD) => theme.syntax(Syntax::Keyword),
        _ => theme.faint,
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum CharClass {
    Word,
    Space,
    Punct,
    Newline,
}

/// The columns where each visible character (grapheme) starts, then the line's end.
fn grapheme_columns(text: &str) -> Vec<usize> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut columns = Vec::new();
    let mut col = 0;
    for g in text.graphemes(true) {
        columns.push(col);
        col += g.chars().count();
    }
    columns.push(col);
    columns
}

fn char_class(c: char) -> CharClass {
    if c == '\n' || c == '\r' {
        // Its own kind, so double-clicking spaces never reaches into the next line.
        CharClass::Newline
    } else if c.is_alphanumeric() || c == '_' {
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
        let explicit = range_utf16.is_some();
        let range = range_utf16
            .map(|r| self.buffer.utf16_to_char(r.start)..self.buffer.utf16_to_char(r.end))
            .or(self.marked.clone())
            .unwrap_or(self.selection.range());
        // With several cursors, plain typing (and finishing an accent) goes to all of them.
        if !self.extra.is_empty() && (!explicit || range == self.selection.range() || self.marked.is_some()) {
            let marked = self.marked.take();
            let text = text.to_string();
            return self.for_each_cursor(cx, |this, cx| {
                let range = match &marked {
                    Some(m) if this.on_primary_cursor() => m.clone(),
                    _ => this.selection.range(),
                };
                let mut chars = text.chars();
                if let (Some(c), None, None) = (chars.next(), chars.next(), &marked)
                    && this.type_pair_char(c, cx)
                {
                    return;
                }
                this.edit(range, &text, EditKind::Typing, cx);
            });
        }
        // Typing what the suggestion shows keeps it.
        let kept =
            if self.marked.is_none() && range == self.selection.range() { self.type_into_ghost(text) } else { None };
        let mut chars = text.chars();
        if let (Some(c), None, None) = (chars.next(), chars.next(), &self.marked)
            && range == self.selection.range()
            && self.type_pair_char(c, cx)
        {
            return;
        }
        self.edit(range, text, EditKind::Typing, cx);
        self.completion_after_typing(text, cx);
        self.signature_after_typing(text, cx);
        self.schedule_ghost(kept, cx);
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
        // Other cursors after it move along with the text being composed.
        let delta = (end - range.start) as isize - range.len() as isize;
        for cursor in &mut self.extra {
            for offset in [&mut cursor.selection.anchor, &mut cursor.selection.head] {
                if *offset >= range.end {
                    *offset = (*offset as isize + delta).max(0) as usize;
                }
            }
        }
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
        if self.completion.is_some() || self.fix_menu.is_some() {
            key_context.add("showing_completions");
        }
        if self.snippet.is_some() {
            key_context.add("in_snippet");
        }
        self.ai_key_context(&mut key_context);
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
            .on_action(cx.listener(|this, _: &IndentWithTabs, _, cx| this.set_indent(IndentStyle::Tabs, cx)))
            .on_action(cx.listener(|this, _: &IndentWith2Spaces, _, cx| this.set_indent(IndentStyle::Spaces(2), cx)))
            .on_action(cx.listener(|this, _: &IndentWith4Spaces, _, cx| this.set_indent(IndentStyle::Spaces(4), cx)))
            .on_action(cx.listener(|this, _: &UseLfLineEndings, _, cx| {
                this.set_line_ending(crate::file_style::LineEnding::Lf, cx)
            }))
            .on_action(cx.listener(|this, _: &UseCrlfLineEndings, _, cx| {
                this.set_line_ending(crate::file_style::LineEnding::Crlf, cx)
            }))
            .on_action(cx.listener(Self::keep_hunk))
            .on_action(cx.listener(Self::undo_hunk))
            .on_action(cx.listener(Self::fold))
            .on_action(cx.listener(Self::unfold))
            .on_action(cx.listener(Self::fold_all))
            .on_action(cx.listener(Self::unfold_all))
            .on_action(cx.listener(Self::go_to_definition))
            .on_action(cx.listener(Self::go_to_type_definition))
            .on_action(cx.listener(Self::go_to_implementation))
            .on_action(cx.listener(Self::show_info))
            .on_action(cx.listener(Self::show_completions))
            .on_action(cx.listener(Self::completion_next))
            .on_action(cx.listener(Self::completion_previous))
            .on_action(cx.listener(Self::confirm_completion))
            .on_action(cx.listener(Self::cancel_completion))
            .on_action(cx.listener(Self::inline_assist))
            .on_action(cx.listener(Self::keep_change_action))
            .on_action(cx.listener(Self::undo_change_action))
            .on_action(cx.listener(Self::close_note_action))
            .on_action(cx.listener(Self::accept_ghost_action))
            .on_action(cx.listener(Self::rename_symbol))
            .on_action(cx.listener(Self::quick_fix))
            .on_action(cx.listener(Self::toggle_breakpoint))
            .on_action(cx.listener(Self::expand_selection))
            .on_action(cx.listener(Self::next_placeholder))
            .on_action(cx.listener(Self::previous_placeholder))
            .on_action(cx.listener(Self::end_snippet))
            .on_action(cx.listener(Self::shrink_selection))
            .on_action(cx.listener(Self::go_to_matching_bracket))
            .on_action(cx.listener(Self::newline_below))
            .on_action(cx.listener(Self::newline_above))
            .on_action(cx.listener(Self::join_lines))
            .on_action(cx.listener(Self::next_change))
            .on_action(cx.listener(Self::previous_change))
            .on_action(cx.listener(Self::sort_lines))
            .on_action(cx.listener(Self::upper_case))
            .on_action(cx.listener(Self::lower_case))
            .on_action(cx.listener(Self::find_references))
            .on_action(cx.listener(Self::format_document))
            .on_action(cx.listener(Self::format_selection))
            .on_action(cx.listener(Self::accept_ghost_word))
            .on_action(cx.listener(Self::accept_ghost_line))
            .on_action(cx.listener(Self::next_ghost))
            .on_action(cx.listener(Self::previous_ghost))
            .on_action(cx.listener(Self::dismiss_ghost_action))
            .on_action(cx.listener(Self::toggle_comment_action))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::move_line_up))
            .on_action(cx.listener(Self::move_line_down))
            .on_action(cx.listener(Self::duplicate_line_up))
            .on_action(cx.listener(Self::duplicate_line_down))
            .on_action(cx.listener(Self::delete_line))
            .on_action(cx.listener(Self::select_line_action))
            .on_action(cx.listener(Self::add_next_occurrence_action))
            .on_action(cx.listener(Self::add_cursors_to_line_ends))
            .on_action(cx.listener(Self::undo_cursor))
            .on_action(cx.listener(Self::select_all_occurrences_action))
            .on_action(cx.listener(Self::add_cursor_above))
            .on_action(cx.listener(Self::add_cursor_below))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(EditorElement::new(cx.entity()));
        let hover = self.render_hover(cx);
        let rename = self.render_rename(cx);
        let completions = self.render_completions(cx);
        let fixes = self.render_fixes(cx);
        let condition = self.render_condition(cx);
        let signature = self.render_signature(cx);
        let ai_blocks = self.render_ai_blocks(cx);
        div()
            .relative()
            .size_full()
            // The AI's rows scroll with the code; clip them to the editor.
            .overflow_hidden()
            .child(text)
            .when_some(find_bar, |editor, bar| editor.child(div().absolute().top(px(8.)).right(px(16.)).child(bar)))
            .children(hover)
            .children(rename)
            .children(completions)
            .children(fixes)
            .children(condition)
            .children(signature)
            .children(ai_blocks)
    }
}
