mod assist;
mod bookmarks;
mod breakpoints;
mod broken_links;
mod changes;
mod color_pick;
mod commands;
mod completion;
mod conflicts;
mod cursors;
mod fixes;
mod fold;
mod ghost;
mod hints;
mod intel;
mod links;
mod mac_keys;
mod marks;
mod refactor;
mod reindent;
mod review;
mod rewrap;
mod signature;
mod snippet;
mod spelling;
mod structure;
mod tags;

pub use assist::{Block, BlockKind};
pub use bookmarks::{NextBookmark, PreviousBookmark, ToggleBookmark};
pub use breakpoints::{Breakpoint, ToggleBreakpoint};
pub use completion::CompletionMenu;

/// Markdown for `html` copied along with `text`, when it's the same words (not left over
/// from an earlier copy) and says more than the plain text does.
fn formatted_paste(html: &str, text: &str) -> Option<String> {
    let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    // Letters only, in lower case: list numbers and CSS's capitals differ between the two.
    let letters =
        |s: &str| s.chars().filter(|c| c.is_alphabetic()).flat_map(char::to_lowercase).take(200).collect::<String>();
    if letters(&crate::html_markdown::plain_text(html)) != letters(text) {
        return None;
    }
    let markdown = crate::html_markdown::markdown(html)?;
    (words(&markdown) != words(text)).then_some(markdown)
}

/// A snippet's text with its variables filled in for `here` (placeholders dropped): for
/// checking what a snippet becomes.
#[cfg(test)]
#[test]
fn formatted_paste_only_when_it_says_more() {
    let html = "<h2>Title</h2><p>A <a href=\"https://x.dev\">link</a></p>";
    assert_eq!(formatted_paste(html, "Title\nA link").as_deref(), Some("## Title\n\nA [link](https://x.dev)"));
    // Plain words only: the text as it is.
    assert_eq!(formatted_paste("<p>just words</p>", "just words"), None);
    // Formatting left from another copy: not these words.
    assert_eq!(formatted_paste(html, "something else"), None);
}

#[cfg(test)]
pub fn fill_snippet(snippet: &str, here: &crate::snippets::Here) -> String {
    snippet::parse_with(snippet, &|name| crate::snippets::variable(name, here)).text
}
pub use conflicts::{Conflict, NextConflict, PreviousConflict};
pub use cursors::Cursor;
pub use fixes::QuickFix;
pub use fold::{Fold, FoldAll, Unfold, UnfoldAll};
pub use ghost::{AcceptGhost, AcceptGhostLine, AcceptGhostWord, NextGhost};
pub use intel::{HoverCard, Problem};
pub use refactor::{
    FindReferences, FormatDocument, FormatSelection, InsertFootnote, InsertTableOfContents, RenameSymbol, apply_edits,
};
pub use review::{KeepHunk, UndoHunk};
pub use rewrap::Rewrap;
pub use structure::{
    CamelCase, ExpandSelection, GoToMatchingBracket, JoinLines, KebabCase, LowerCase, NewlineAbove, NewlineBelow,
    NextChange, PascalCase, PreviousChange, RemoveDuplicateLines, ReverseLines, ShrinkSelection, SnakeCase, SortLines,
    TitleCase, UpperCase,
};

use crate::buffer::Buffer;
use crate::element::{EditorElement, RowLayout};
use crate::file_style::Indent as IndentStyle;
use crate::find_bar::{CloseFind, DeployFind, DeployReplace, FindBar, FindNext, FindPrevious, UseSelectionForFind};
use crate::fonts::CodeFont;
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
use std::rc::Rc;
use std::time::{Duration, Instant};

actions!(markdown_preview, [ToggleMarkdownPreview]);

actions!(
    file_style,
    [IndentWithTabs, IndentWith2Spaces, IndentWith4Spaces, UseLfLineEndings, UseCrlfLineEndings, UseUtf8Encoding]
);

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
        PasteAsIs,
        ToggleTask,
        Undo,
        Redo,
        Save,
        GoToDefinition,
        PeekDefinition,
        ExpandMacro,
        GoToParentModule,
        OpenCargoToml,
        GoToTypeDefinition,
        GoToImplementation,
        ShowCallers,
        ShowInfo,
        ShowCompletions,
        CompletionNext,
        CompletionPrevious,
        ConfirmCompletion,
        CancelCompletion,
        InlineAssist,
        ToggleComment,
        ToggleBlockComment,
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
    conflicts::bind_keys(cx);
    breakpoints::bind_keys(cx);
    bookmarks::bind_keys(cx);
    structure::bind_keys(cx);
    snippet::bind_keys(cx);
    mac_keys::bind_keys(cx);
}

pub fn bind_ai_keys(cx: &mut App) {
    assist::bind_keys(cx);
    review::bind_keys(cx);
    ghost::bind_keys(cx);
}

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Editor");
    cx.bind_keys([
        KeyBinding::new("secondary-shift-v", ToggleMarkdownPreview, ctx),
        KeyBinding::new("escape", ToggleMarkdownPreview, Some("MarkdownPreview")),
    ]);
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
        KeyBinding::new("alt-shift-secondary-v", PasteAsIs, ctx),
        KeyBinding::new("secondary-shift-x", ToggleTask, ctx),
        KeyBinding::new("secondary-z", Undo, ctx),
        KeyBinding::new("secondary-shift-z", Redo, ctx),
        KeyBinding::new("secondary-s", Save, ctx),
        KeyBinding::new("f12", GoToDefinition, ctx),
        KeyBinding::new("alt-f12", PeekDefinition, ctx),
        KeyBinding::new("secondary-f12", GoToImplementation, ctx),
        KeyBinding::new("ctrl-alt-h", ShowCallers, ctx),
        KeyBinding::new("secondary-shift-i", ShowInfo, ctx),
        KeyBinding::new("ctrl-space", ShowCompletions, ctx),
        KeyBinding::new("secondary-i", InlineAssist, ctx),
        KeyBinding::new("secondary-/", ToggleComment, ctx),
        KeyBinding::new("alt-secondary-/", ToggleBlockComment, ctx),
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
    /// Save was asked for, but the file changed on disk under the unsaved edits: saving
    /// would write over that, so it waits for an answer.
    SaveConflict,
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
    /// Files dropped from the Finder on a Markdown file, at char offset `at`: the workspace
    /// puts links to them there.
    FilesDropped {
        paths: Vec<PathBuf>,
        at: usize,
    },
    /// The breakpoints changed (set, removed, or moved by an edit).
    BreakpointsChanged,
    /// The bookmarks changed (set, removed, or moved by an edit).
    BookmarksChanged,
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
    CodeAction(Box<lsp_types::CodeActionOrCommand>),
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
    encoding: crate::encoding::Encoding,
    view: (usize, usize, usize),
    bookmarks: Vec<usize>,
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
    /// Colours for the lines around the view (see [`Self::highlight_bytes`]).
    pub spans: Vec<Span>,
    /// The longest line's width in columns, for the buffer revision it was measured at.
    longest_line: std::cell::Cell<(u64, usize)>,
    /// Set for an image or a file that isn't text, shown instead of the text.
    pub preview: Option<crate::preview::Preview>,
    /// The file was deleted on disk while open (a checkout, the terminal): the text is
    /// still here, and saving puts the file back.
    pub missing: bool,
    /// The file changed on disk under unsaved edits here (or under ones brought back from
    /// last time): a save asks before writing over it.
    pub disk_changed: bool,
    /// What the file last held on disk, as far as Null knows, to recognise it moved.
    pub on_disk: Option<Fingerprint>,
    /// How the file's bytes are text, kept when saving.
    pub encoding: crate::encoding::Encoding,
    /// The view's height when last drawn, to keep the caret in view as it shrinks.
    pub viewport_height: Option<f32>,
    /// Markdown shown as it reads (⌘⇧V) instead of its source.
    pub reading: bool,
    /// The parsed document for the preview, and the revision it's of.
    #[allow(clippy::type_complexity)]
    markdown: Option<(u64, Rc<Vec<(usize, crate::markdown_view::Block)>>)>,
    /// The source line the preview last scrolled to, following the other side.
    followed_line: Option<usize>,
    reading_scroll: gpui::ScrollHandle,
    /// The merge conflicts in the text, and the revision they were found at.
    conflicts: std::cell::RefCell<(u64, std::rc::Rc<[Conflict]>)>,
    /// The buffer revision and byte range `spans` cover.
    spans_for: Option<(u64, Range<usize>)>,
    /// `problems()` for a (diagnostics version, buffer revision).
    #[allow(clippy::type_complexity)]
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
    /// The next scroll to the caret puts its line in the middle (⌃L).
    pub center_once: bool,
    /// The tag pair outlined, for a (revision, caret).
    #[allow(clippy::type_complexity)]
    tag_pair_seen: Option<((u64, usize), Option<[Range<usize>; 2]>)>,
    dragging: Option<DragUnit>,
    /// Where dragged text would land, shown as a caret while it's dragged.
    pub drop_at: Option<usize>,
    pub font_size: Pixels,
    /// A line's height as a multiple of the font size (the "Line spacing" setting).
    line_spacing: f32,
    /// The words in a prose file, for the revision counted.
    words: std::cell::Cell<Option<(u64, usize)>>,
    /// Which lines are in Markdown fences, as of a version of the text (for spelling).
    fences: std::cell::RefCell<Option<(u64, Vec<bool>)>>,
    /// Whether linked files exist, and since when that's known (for broken links).
    link_targets: std::cell::RefCell<std::collections::HashMap<PathBuf, (bool, Instant)>>,
    /// The `#anchors` the headings make, as of a version of the text.
    anchors: std::cell::RefCell<Option<(u64, Vec<String>)>>,
    /// The words changed in the changes shown, for the version, blocks and lines they were
    /// worked out for (see `word_changes`).
    #[allow(clippy::type_complexity)]
    word_changes:
        std::cell::RefCell<Option<((u64, Vec<(usize, usize)>, Range<usize>), std::rc::Rc<review::WordChanges>)>>,
    pub search: Option<SearchState>,
    /// Find looks only here (bytes, as of a revision of the text): the lines selected when
    /// the find bar opened. It follows edits, Replace All's own included.
    find_scope: Option<(Range<usize>, u64)>,
    /// Where the scope was at each version of the text, so an undo (which brings an
    /// earlier version back whole) brings its scope back too.
    find_scope_at: Vec<(u64, Range<usize>)>,
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
    /// Where the caret was after the last ⌃K, so another adds to what was cut.
    killed_at: Option<(u64, usize)>,
    /// A tag's name and its pair's, edited together.
    linked: Option<tags::LinkedTag>,
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
    /// Lines (from 0) marked to come back to; they move with edits.
    pub bookmarks: Vec<usize>,
    bookmarks_revision: u64,
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
    /// The mouse is over a changed line's mark in the gutter (a click opens the change), or
    /// a color's square (a click picks another).
    over_change_mark: bool,
    /// A color being picked in the Mac's color panel.
    color_pick: Option<color_pick::ColorPick>,
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

/// A file's text in brief: its length and a hash, enough to tell it again somewhere else.
pub type Fingerprint = (usize, u64);

pub fn fingerprint(text: &str) -> Fingerprint {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    text.hash(&mut hasher);
    (text.len(), hasher.finish())
}

/// Colours for a file, unless it's too big to colour as you type: a minified bundle's one
/// long line, or a file of many megabytes, is parsed again after every keystroke, which
/// took longer than the keystroke. Those show as plain text, as in other editors.
fn highlighter_for(path: &std::path::Path, buffer: &Buffer) -> Option<Highlighter> {
    const MAX_COLOURED_LINE: usize = 20_000;
    const MAX_COLOURED_FILE: usize = 8 * 1024 * 1024;
    let language = languages::for_path(path)?;
    let rope = buffer.rope();
    if rope.len_bytes() > MAX_COLOURED_FILE || rope.lines().any(|l| l.len_bytes() > MAX_COLOURED_LINE) {
        return None;
    }
    Highlighter::new(language)
}

impl Editor {
    pub fn new(buffer: Buffer, path: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let highlighter = path.as_deref().and_then(|p| highlighter_for(p, &buffer));
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
            preview: None,
            missing: false,
            disk_changed: false,
            on_disk: None,
            encoding: Default::default(),
            viewport_height: None,
            reading: false,
            markdown: None,
            followed_line: None,
            reading_scroll: gpui::ScrollHandle::new(),
            conflicts: std::cell::RefCell::new((u64::MAX, std::rc::Rc::from([]))),
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
            center_once: false,
            tag_pair_seen: None,
            dragging: None,
            drop_at: None,
            font_size: px(cx.global::<Settings>().font_size),
            line_spacing: cx.global::<Settings>().line_spacing.factor(),
            words: Default::default(),
            fences: Default::default(),
            link_targets: Default::default(),
            anchors: Default::default(),
            word_changes: Default::default(),
            search: None,
            find_scope: None,
            find_scope_at: Vec::new(),
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
            linked: None,
            killed_at: None,
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
            bookmarks: Vec::new(),
            bookmarks_revision: 0,
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
            over_change_mark: false,
            color_pick: None,
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
        let read = crate::encoding::read(&path);
        // An image, or a file that isn't text: shown, never edited or saved over.
        if let Some(preview) = crate::preview::of(&path, &read.as_ref().map(|(t, _)| t.as_str()).map_err(|e| e.kind()))
        {
            let mut editor = Self::new(Buffer::new(), Some(path), cx);
            editor.preview = Some(preview);
            return editor;
        }
        let (text, encoding) = read.unwrap_or_default();
        let mut editor = Self::new(Buffer::from_text(&text), Some(path), cx);
        editor.on_disk = Some(fingerprint(&text));
        editor.encoding = encoding;
        editor.reload_git_base(cx);
        if let Some(lsp) = lsp {
            editor.attach_lsp(lsp, cx);
        }
        editor
    }

    /// Picks up a change made to the file outside Null. Unsaved edits are never
    /// overwritten; the reload itself can be undone.
    pub fn reload_from_disk(&mut self, cx: &mut Context<Self>) {
        self.reload(false, cx);
    }

    /// The file as it is on disk now, unsaved edits or not (after they were discarded).
    pub fn revert_to_disk(&mut self, cx: &mut Context<Self>) {
        self.reload(true, cx);
    }

    fn reload(&mut self, discard_edits: bool, cx: &mut Context<Self>) {
        if self.check_missing(cx) {
            return;
        }
        let Some(path) = &self.path else { return };
        let read = crate::encoding::read(path);
        if self.preview.is_some() {
            let text = read.as_ref().map(|(t, _)| t.as_str()).map_err(|e| e.kind());
            let now = crate::preview::of(path, &text);
            // Readable now (its permissions were fixed): it opens as text after all.
            let readable_now = now.is_none() && read.is_ok();
            if !(readable_now && matches!(self.preview, Some(crate::preview::Preview::Unreadable { .. }))) {
                self.preview = now.or(self.preview.take());
                return cx.notify();
            }
            self.preview = None;
        }
        let Ok((text, encoding)) = read else { return };
        // What Null itself last wrote (the watcher saw a save, typing has gone on since):
        // nothing changed on disk, unless another app wrote it in another encoding.
        if !discard_edits && self.on_disk == Some(fingerprint(&text)) {
            self.encoding = encoding;
            return;
        }
        self.on_disk = Some(fingerprint(&text));
        self.encoding = encoding;
        if text == self.buffer.to_string() {
            self.disk_changed = false;
            return;
        }
        if self.buffer.is_dirty() && !discard_edits {
            self.disk_changed = true;
            cx.emit(EditorEvent::ChangedOnDisk);
            return;
        }
        self.disk_changed = false;
        let (line, column) = self.caret_point();
        self.record_undo(EditKind::Other);
        self.buffer.replace(0..self.buffer.len_chars(), &text);
        self.buffer.mark_saved();
        self.single_cursor();
        self.selection = Selection::caret(self.buffer.offset(line, column));
        self.text_changed(cx);
        cx.notify();
    }

    /// Notes whether the file is gone from disk; true when it is.
    pub fn check_missing(&mut self, cx: &mut Context<Self>) -> bool {
        let missing = self.path.as_ref().is_some_and(|p| !p.exists());
        if missing != self.missing {
            self.missing = missing;
            cx.notify();
        }
        missing
    }

    /// Gives the buffer a new file (after a rename, or the first save of an untitled file).
    pub fn set_path(&mut self, path: PathBuf, lsp: Option<Entity<LspStore>>, cx: &mut Context<Self>) {
        self.missing = false;
        self.disk_changed = false;
        self.release_lsp(cx);
        self.highlighter = highlighter_for(&path, &self.buffer);
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
        (self.font_size * self.line_spacing).round()
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
        self.bookmarks_after_edit(cx);
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
        const BIG_FILE: usize = 4 * 1024 * 1024;
        let width = |line: ropey::RopeSlice| {
            // A very long line counts a column a character, as it's drawn (see `wrap::LONG_LINE`).
            if line.len_chars() > crate::wrap::LONG_LINE {
                return line.len_chars();
            }
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
            // A big file is measured a column a character: counting columns through every
            // character of a 70 MB log took half a second, on opening and after each undo.
            // The rows on screen are measured as drawn, so a wide one still scrolls fully.
            None if rope.len_bytes() > BIG_FILE => rope.lines().map(|l| l.len_chars()).max().unwrap_or(0),
            None => rope.lines().map(width).max().unwrap_or(0),
        };
        self.longest_line.set((self.buffer.revision(), cols));
        cols
    }

    /// Makes sure `spans` colours `lines`: the syntax tree catches up with the edits
    /// (only what changed is parsed again), then the lines around the view are
    /// coloured, with some room so scrolling a little needs nothing new.
    /// Makes sure `spans` colours the bytes `wanted` (what's on screen): the syntax tree
    /// catches up with the edits (only what changed is parsed again), then the lines
    /// around are coloured, with some room so scrolling a little needs nothing new.
    pub(crate) fn highlight_bytes(&mut self, wanted: Range<usize>) {
        /// Lines coloured beyond the view on each side, so scrolling doesn't colour again…
        const ROOM: usize = 120;
        /// …but after an edit, only a few: what's on screen is what's needed now (colouring
        /// all the room took 3 ms a keystroke on a big file).
        const ROOM_AFTER_EDIT: usize = 10;
        /// The room never goes past this many bytes either side: on a minified file's one
        /// long line, a few lines of room were the whole file.
        const MAX_ROOM_BYTES: usize = 32 * 1024;
        let Some(highlighter) = &mut self.highlighter else { return };
        let revision = self.buffer.revision();
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
        let rope = self.buffer.rope();
        let len = rope.len_bytes();
        let first = rope.byte_to_line(wanted.start.min(len));
        let last = rope.byte_to_line(wanted.end.min(len));
        let start =
            self.buffer.line_to_byte(first.saturating_sub(room)).max(wanted.start.saturating_sub(MAX_ROOM_BYTES));
        let end = self.buffer.line_to_byte(last + 1 + room).min(wanted.end + MAX_ROOM_BYTES).min(len);
        let range = start..end.max(wanted.end.min(len));
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

    /// ⌘E: the selection (or the word at the caret) becomes what's searched for, its matches
    /// shown and ⌘G ⌘⇧G going through them, the find bar open or not.
    fn use_selection_for_find(&mut self, _: &UseSelectionForFind, window: &mut Window, cx: &mut Context<Self>) {
        let range = if self.selection.is_empty() { self.word_at(self.selection.head) } else { self.selection.range() };
        let text = self.buffer.slice(range.clone());
        if text.is_empty() || text.contains('\n') {
            return;
        }
        crate::find_bar::remember_search(&text, cx);
        if let Some(bar) = self.find_bar.clone() {
            return bar.update(cx, |bar, cx| bar.show(Some(text), false, window, cx));
        }
        // The word itself is the match it's on, so ⌘G goes on to the next.
        self.selection = Selection { anchor: range.start, head: range.end };
        let query = SearchQuery { text, regex: false, ..self.last_query.clone() };
        self.set_search(query, cx);
    }

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

    /// Where find looks, brought up to the current text: None for the whole file.
    fn find_scope_bytes(&mut self) -> Option<Range<usize>> {
        let (range, revision) = self.find_scope.clone()?;
        let revision_now = self.buffer.revision();
        let version = self.buffer.version();
        if revision != revision_now {
            let moved = match self.buffer.edits_since(revision) {
                Some(mut edits) => edits.try_fold(range, intel::map_range),
                // After an undo: where it was at that version; not known, the whole file.
                None => self.find_scope_at.iter().rev().find(|(v, _)| *v == version).map(|(_, r)| r.clone()),
            };
            self.find_scope = moved.clone().map(|r| (r, revision_now));
            if let Some(range) = &moved {
                self.remember_find_scope(version, range.clone());
            }
            return moved;
        }
        Some(range)
    }

    fn remember_find_scope(&mut self, version: u64, range: Range<usize>) {
        if self.find_scope_at.last().is_some_and(|(v, _)| *v == version) {
            self.find_scope_at.pop();
        }
        self.find_scope_at.push((version, range));
        if self.find_scope_at.len() > 64 {
            self.find_scope_at.remove(0);
        }
    }

    /// Whether find looks only in the lines selected when it opened.
    pub fn find_in_selection(&self) -> bool {
        self.find_scope.is_some()
    }

    /// Find looks in the whole file again.
    pub fn find_in_whole_file(&mut self, cx: &mut Context<Self>) {
        self.find_scope = None;
        self.refresh_search();
        cx.notify();
    }

    fn refresh_search(&mut self) {
        let scope = self.find_scope_bytes();
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
            .find_within(regex, &text, scope.clone())
            .into_iter()
            .map(|r| rope.byte_to_char(r.start)..rope.byte_to_char(r.end))
            .collect();
        let from = match &scope {
            Some(s) => rope.byte_to_char(s.start),
            None => self.selection.range().start,
        };
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

    /// What ⌘G and ⌘F start from: the latest search in any file (as the Mac shares one
    /// between apps), else this file's own.
    fn known_query(&self, cx: &App) -> SearchQuery {
        match crate::find_bar::latest_search(cx) {
            // Searched here: with this file's choices (case, word, regex).
            Some(text) if text == self.last_query.text => self.last_query.clone(),
            // From another file: as plain text (its choices there aren't known here).
            Some(text) => SearchQuery { text, ..SearchQuery::default() },
            None => self.last_query.clone(),
        }
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.search.is_none() {
            let query = self.known_query(cx);
            if query.text.is_empty() {
                return;
            }
            let before = self.selection.range();
            self.set_search(query, cx);
            // Already on that match (the find bar was just closed on it): on to the next.
            if self.selection.range() != before {
                return;
            }
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
        let scope = self.find_scope_bytes();
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
            .filter(|r| scope.as_ref().is_none_or(|s| s.start <= r.start && r.end <= s.end))
            .map(|bytes| {
                let chars = rope.byte_to_char(bytes.start)..rope.byte_to_char(bytes.end);
                (chars, search.query.replacement_for(regex, &text, bytes, replacement))
            })
            .collect();
        if edits.is_empty() {
            return;
        }
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
        let prefill = (!selected.is_empty() && !selected.contains('\n')).then_some(selected.clone());
        // Lines selected: find looks only there (a word or none: the whole file).
        self.find_scope = selected.contains('\n').then(|| {
            let range = self.selection.range();
            let rope = self.buffer.rope();
            (rope.char_to_byte(range.start)..rope.char_to_byte(range.end), self.buffer.revision())
        });
        self.find_scope_at.clear();
        if let Some((range, _)) = self.find_scope.clone() {
            self.remember_find_scope(self.buffer.version(), range);
        }
        if let Some(bar) = self.find_bar.clone() {
            bar.update(cx, |bar, cx| bar.show(prefill, replace, window, cx));
            self.refresh_search();
            cx.notify();
            return;
        }
        let known = self.known_query(cx);
        let query = SearchQuery { text: prefill.unwrap_or_else(|| known.text.clone()), ..known };
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
        self.find_scope = None;
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
        // The caret moved by the keyboard (or an edit): the view follows it all the way, as
        // typewriter scrolling has it. A click sets `reveal_only` after this.
        self.reveal_only = false;
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
        // Typing in a tag's name types in its pair's too.
        let linked = matches!(kind, EditKind::Typing | EditKind::Deleting).then(|| self.linked_tag(&range)).flatten();
        self.record_undo(kind);
        let mut end = self.buffer.replace(range.clone(), text);
        if let Some(linked) = linked {
            end = self.mirror_tag(linked, range.clone(), end - range.start, end);
        }
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
            if this.continue_markdown(range.clone(), line, col, &line_text, cx)
                || this.continue_comment(range.clone(), line, col, &line_text, cx)
            {
                return;
            }
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
            // Between a tag and its closing one (`<div>|</div>`), as between braces.
            let between_tags = matches!(this.language_name(), "HTML" | "JavaScript" | "TSX")
                && before == Some('>')
                && line_text.chars().skip(col).collect::<String>().trim_start().starts_with("</")
                && {
                    let head: String = line_text.chars().take(col).collect();
                    head.rfind('<').is_some_and(|lt| !head[lt..].starts_with("</") && !head.ends_with("/>"))
                };
            let opens = matches!(before, Some('{' | '(' | '[')) || python_block || between_tags;
            // Python: after `return`, `pass`, `break`, `continue` or `raise`, the block is over.
            let ends_block = this.language_name() == "Python" && {
                let word =
                    line_text.trim_start().split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or("");
                matches!(word, "return" | "pass" | "break" | "continue" | "raise")
                    && line_text.chars().skip(col).all(char::is_whitespace)
                    && !opens
            };
            let unit = this.style.indent.unit();
            let indent = match indent.strip_suffix(unit.as_str()) {
                Some(out) if ends_block => out.to_string(),
                _ => indent,
            };
            let inner = format!("{indent}{}", if opens { unit } else { String::new() });
            // The file's own line break: "\r\n" in a Windows file.
            let nl = this.style.line_ending.text();
            if opens
                && (between_tags
                    || matches!(
                        (before, after),
                        (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))
                    ))
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

    /// Enter in a Markdown list or quote: the next line starts the next item (an empty
    /// item ends the list instead). False when it isn't one, or the caret is in code.
    fn continue_markdown(
        &mut self,
        range: Range<usize>,
        line: usize,
        col: usize,
        line_text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_markdown() || !range.is_empty() {
            return false;
        }
        let before: String = line_text.chars().take(col).collect();
        let Some((next, empty)) = crate::markdown_view::continuation(&before) else { return false };
        // Inside a fenced code block, Enter is just Enter.
        let fences = (0..line).filter(|&l| self.buffer.line_text(l).trim_start().starts_with("```")).count();
        if fences % 2 == 1 {
            return false;
        }
        let start = self.buffer.line_to_char(line);
        if empty && line_text.chars().skip(col).all(char::is_whitespace) {
            // An empty item: the marker goes, and the list ends here.
            self.edit(start..start + self.buffer.line_len(line), "", EditKind::Other, cx);
            return true;
        }
        // A list numbered all the same on purpose (`1.` `1.`) goes on that way.
        let next = match self.list_lines(line).and_then(|(lines, at)| crate::markdown_view::same_number(&lines, at)) {
            Some(n) => {
                let indent = next.len() - next.trim_start().len();
                let digits = next[indent..].chars().take_while(char::is_ascii_digit).count();
                if digits > 0 { format!("{}{n}{}", &next[..indent], &next[indent + digits..]) } else { next }
            }
            None => next,
        };
        let nl = self.style.line_ending.text();
        self.edit(range, &format!("{nl}{next}"), EditKind::Other, cx);
        // A numbered item added in the middle: the ones after it count on.
        self.renumber_list(line + 1, cx);
        true
    }

    /// The lines near `line` (a list isn't thousands of lines long), and where `line` is
    /// among them.
    fn list_lines(&self, line: usize) -> Option<(Vec<String>, usize)> {
        let window = line.saturating_sub(2000)..(line + 2000).min(self.buffer.len_lines());
        (line < window.end).then(|| (window.clone().map(|l| self.buffer.line_text(l)).collect(), line - window.start))
    }

    /// Numbered Markdown lists around `line` brought back in order (see
    /// `markdown_view::renumbered`), as part of the edit just made: one undo takes back both.
    pub(super) fn renumber_list(&mut self, line: usize, cx: &mut Context<Self>) {
        if !self.is_markdown() || self.in_fence(line) {
            return;
        }
        let Some((lines, at)) = self.list_lines(line) else { return };
        let start_line = line - at;
        let changes = crate::markdown_view::renumbered(&lines, at);
        if changes.is_empty() {
            return;
        }
        let point = |o: usize| self.buffer.point(o);
        let (mut anchor, mut head) = (point(self.selection.anchor), point(self.selection.head));
        for (i, text) in changes.iter().rev() {
            let l = start_line + i;
            let old_len = self.buffer.line_len(l);
            let start = self.buffer.line_to_char(l);
            self.buffer.replace(start..start + old_len, text);
            let grew = text.chars().count() as isize - old_len as isize;
            // The selection's ends after the number move with the line's text.
            for end in [&mut anchor, &mut head] {
                if end.0 == l && end.1 > 0 {
                    end.1 = (end.1 as isize + grew).max(0) as usize;
                }
            }
        }
        let offset = |(l, c): (usize, usize)| self.buffer.offset(l, c);
        self.selection = Selection { anchor: offset(anchor), head: offset(head) };
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        cx.notify();
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
        // In a Markdown table, Tab goes to the next cell.
        if self.table_step(true, cx) {
            return;
        }
        // In a Markdown list item, Tab nests the item, wherever the caret is in it.
        if self.is_markdown() {
            let line = self.buffer.point(self.selection.head).0;
            let text = self.buffer.line_text(line);
            let is_item =
                crate::markdown_view::continuation(&text).is_some_and(|(next, _)| !next.trim().starts_with('>'));
            if is_item {
                return self.indent_lines(cx);
            }
        }
        // In HTML, an abbreviation before the caret (`ul>li*3`) becomes its tags.
        if self.expand_abbreviation(cx) {
            return;
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
        self.paste_clipboard(true, cx);
    }

    /// Pastes code as it was copied, without moving it to the indentation where it goes.
    fn paste_as_is(&mut self, _: &PasteAsIs, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_clipboard(false, cx);
    }

    fn paste_clipboard(&mut self, adjust: bool, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else { return };
        // An image (a screenshot) pasted in Markdown: saved next to the file, linked here.
        if self.is_markdown() && item.text().is_none() {
            let image = item.entries().iter().find_map(|entry| match entry {
                gpui::ClipboardEntry::Image(image) => Some(image.clone()),
                _ => None,
            });
            if let Some(image) = image {
                return self.paste_image(image, cx);
            }
        }
        let Some(text) = item.text() else { return };
        // A web address pasted over some words in Markdown links them: [words](address).
        if adjust
            && self.is_markdown()
            && self.extra.is_empty()
            && let Some(link) = markdown_link(&self.buffer.slice(self.selection.range()), text.trim())
        {
            return self.edit(self.selection.range(), &link, EditKind::Other, cx);
        }
        // Cells copied from a spreadsheet, pasted in Markdown: a table, on lines of its own
        // (not in a code fence: that's code).
        if adjust
            && self.is_markdown()
            && self.extra.is_empty()
            && !self.in_fence(self.buffer.point(self.selection.range().start).0)
            && let Some(table) = crate::markdown_view::table_from_cells(&text)
        {
            let (_, column) = self.buffer.point(self.selection.range().start);
            let table = if column > 0 { format!("\n\n{table}") } else { table };
            let table = match self.style.line_ending {
                crate::file_style::LineEnding::Crlf => table.replace('\n', "\r\n"),
                crate::file_style::LineEnding::Lf => table,
            };
            return self.edit(self.selection.range(), &table, EditKind::Other, cx);
        }
        // Copied from a web page or a document, pasted in Markdown: its formatting as
        // Markdown (headings, links, bold, lists…). ⌥⇧⌘V pastes the plain text.
        if adjust
            && self.is_markdown()
            && self.extra.is_empty()
            && !self.in_fence(self.buffer.point(self.selection.range().start).0)
            && let Some(markdown) =
                crate::html_markdown::clipboard_html().and_then(|html| formatted_paste(&html, &text))
        {
            // Blocks (headings, lists…) go on lines of their own; a phrase goes in the line.
            let range = self.selection.range();
            let (_, column) = self.buffer.point(range.start);
            let (end_line, end_column) = self.buffer.point(range.end);
            let first = markdown.lines().next().unwrap_or("");
            let numbered = first
                .split_once(['.', ')'])
                .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            let blocks = markdown.contains('\n') || numbered || first.starts_with(['#', '-', '>', '|', '`', '~']);
            let text_after = end_column < self.buffer.line_len(end_line);
            let markdown = match (blocks && column > 0, blocks && text_after) {
                (true, true) => format!("\n\n{markdown}\n\n"),
                (true, false) => format!("\n\n{markdown}"),
                (false, true) => format!("{markdown}\n\n"),
                (false, false) => markdown,
            };
            let markdown = match self.style.line_ending {
                crate::file_style::LineEnding::Crlf => markdown.replace('\n', "\r\n"),
                crate::file_style::LineEnding::Lf => markdown,
            };
            return self.edit(self.selection.range(), &markdown, EditKind::Other, cx);
        }
        let kind = item.metadata().cloned().unwrap_or_default();
        // Pasted line breaks become the file's own.
        let text = text.replace("\r\n", "\n");
        let text = match self.style.line_ending {
            crate::file_style::LineEnding::Crlf => text.replace('\n', "\r\n"),
            crate::file_style::LineEnding::Lf => text,
        };
        self.paste_text(text, &kind, adjust, cx);
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
        editor.encoding = source.encoding;
        editor.lsp_follower = true;
        if let Some(lsp) = lsp {
            editor.attach_lsp(lsp, cx);
        }
        editor.reload_git_base(cx);
        let (line, column, top) = source.view;
        editor.restore_view(line, column, top, cx);
        editor.set_bookmarks(source.bookmarks, cx);
        editor
    }

    /// What a second copy of this editor's file starts from.
    pub fn twin_source(&self) -> Option<TwinSource> {
        Some(TwinSource {
            text: self.buffer.to_string(),
            dirty: self.buffer.is_dirty(),
            path: self.path.clone()?,
            style: self.style.clone(),
            encoding: self.encoding,
            view: self.view_state(),
            bookmarks: self.bookmarks.clone(),
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

    /// An image from the clipboard saved next to this Markdown file as
    /// `pasted-2026-10-06-120312.png` (UTC), and a link to it in place of the selection.
    fn paste_image(&mut self, image: gpui::Image, cx: &mut Context<Self>) {
        use gpui::ImageFormat as F;
        let at = self.selection.head;
        let Some(dir) = self.path.as_deref().and_then(Path::parent).map(Path::to_path_buf) else {
            return self.show_notice(at, "Save the file first: the image goes next to it.".into(), cx);
        };
        let ext = match image.format {
            F::Png => "png",
            F::Jpeg => "jpg",
            F::Webp => "webp",
            F::Gif => "gif",
            F::Svg => "svg",
            F::Bmp => "bmp",
            F::Tiff => "tiff",
        };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let stamp = utc_stamp(now.as_secs());
        let Some(target) = (1..100)
            .map(|n| if n == 1 { format!("pasted-{stamp}.{ext}") } else { format!("pasted-{stamp}-{n}.{ext}") })
            .map(|name| dir.join(name))
            .find(|p| !p.exists())
        else {
            return;
        };
        if let Err(error) = std::fs::write(&target, &image.bytes) {
            return self.show_notice(at, format!("Couldn't save the image: {error}"), cx);
        }
        let link = crate::markdown_view::link_to(&dir, &target);
        self.single_cursor();
        self.edit(self.selection.range(), &link, EditKind::Other, cx);
    }

    /// Toggle Task (⇧⌘X) in Markdown, on the caret's line or every selected one: a task
    /// ticked or unticked, a list item or a line made a task. Selected lines all tick, or all
    /// untick, as the first one goes.
    fn toggle_task(&mut self, _: &ToggleTask, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_markdown() {
            return;
        }
        let range = self.selection.range();
        let (first, _) = self.buffer.point(range.start);
        let (mut last, last_col) = self.buffer.point(range.end);
        if last > first && last_col == 0 {
            last -= 1;
        }
        let mut edits = Vec::new();
        let mut tick: Option<bool> = None;
        for line in first..=last {
            let text = self.buffer.line_text(line);
            // Code, and the fences around it, aren't tasks.
            if self.in_fence(line) || crate::markdown_view::fence_of(text.trim_start()).is_some() {
                continue;
            }
            let Some(mut new) = crate::markdown_view::task_toggled(&text) else { continue };
            // The first line says which way they all go.
            let ticked = |t: &str| crate::markdown_view::task_box(t).is_some_and(|b| t[b].contains(['x', 'X']));
            let wanted = *tick.get_or_insert(ticked(&new));
            if ticked(&new) != wanted
                && let Some(again) = crate::markdown_view::task_toggled(&new)
            {
                new = again;
            }
            if new != text {
                let start = self.buffer.line_to_char(line);
                edits.push((start..start + self.buffer.line_len(line), new));
            }
        }
        let selection = self.selection;
        let (line, col) = self.caret_point();
        let before = self.buffer.line_len(line);
        self.apply_char_edits(edits, cx);
        // The caret stays by its text; a selection stays on its lines.
        if selection.is_empty() {
            let grew = self.buffer.line_len(line) as isize - before as isize;
            let col = if col == 0 { 0 } else { (col as isize + grew).max(0) as usize };
            self.selection = Selection::caret(self.buffer.offset(line, col));
        } else {
            let start = self.buffer.line_to_char(first);
            let end = self.buffer.line_to_char(last) + self.buffer.line_len(last);
            self.selection = Selection { anchor: start, head: end };
        }
        cx.notify();
    }

    /// A box clicked in the preview: the `task`th of the `drawn` ones, ticked or unticked in
    /// the text. Left alone if the text doesn't show as many (it changed, or reads differently).
    fn tick_task(&mut self, task: usize, drawn: usize, cx: &mut Context<Self>) {
        let lines = crate::markdown_view::task_lines(&self.buffer.to_string());
        if lines.len() != drawn {
            return;
        }
        let Some(&line) = lines.get(task) else { return };
        let Some(ticked) = crate::markdown_view::toggle_task(&self.buffer.line_text(line)) else { return };
        let start = self.buffer.line_to_char(line);
        let end = start + self.buffer.line_len(line);
        let selection = self.selection;
        self.edit(start..end, &ticked, EditKind::Other, cx);
        // Reading, the caret stays where it was.
        self.selection = selection;
    }

    /// How many words a prose file has (counted again only after it changes); None for a
    /// file too big to count on every keystroke.
    pub fn word_count(&self) -> Option<usize> {
        let revision = self.buffer.revision();
        if let Some((counted, words)) = self.words.get()
            && counted == revision
        {
            return Some(words);
        }
        if self.buffer.rope().len_bytes() > WORD_COUNT_BYTES {
            return None;
        }
        let words = self.buffer.rope().chunks().fold((0, false), |(n, carried), chunk| count_words(chunk, n, carried));
        let words = words.0 + usize::from(words.1);
        self.words.set(Some((revision, words)));
        Some(words)
    }

    /// Text put in at char offset `at`, as one undo step, the caret after it.
    pub fn insert_at(&mut self, at: usize, text: &str, cx: &mut Context<Self>) {
        let at = at.min(self.buffer.len_chars());
        self.single_cursor();
        self.edit(at..at, text, EditKind::Other, cx);
    }

    /// Switches the file's line endings, converting every line break.
    /// The test the caret is in (or with `at_caret` false, the file's tests), as a command.
    pub fn test_run(&mut self, root: &std::path::Path, at_caret: bool) -> Option<crate::test_at::TestRun> {
        let path = self.path.clone()?;
        let language = self.language()?.name;
        let highlighter = self.highlighter.as_mut()?;
        highlighter.sync(&self.buffer);
        let tree = highlighter.tree()?.clone();
        let byte = at_caret.then(|| self.buffer.rope().char_to_byte(self.selection.head));
        crate::test_at::find(language, root, &path, &self.buffer.to_string(), &tree, byte)
    }

    /// Saves the file as UTF-8 from now on: the text stays, its bytes change on the next save.
    pub fn use_utf8(&mut self, cx: &mut Context<Self>) {
        if self.encoding != crate::encoding::Encoding::Utf8 && self.preview.is_none() {
            self.encoding = crate::encoding::Encoding::Utf8;
            self.buffer.mark_unsaved();
            cx.emit(EditorEvent::Edited);
            cx.notify();
        }
    }

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

    /// Saves over a file that changed on disk, once that was asked.
    pub fn overwrite_disk(&mut self, cx: &mut Context<Self>) -> bool {
        self.disk_changed = false;
        self.save_to_disk(cx)
    }

    pub fn save_to_disk(&mut self, cx: &mut Context<Self>) -> bool {
        // Nothing to write: what's shown is the file itself.
        if self.preview.is_some() {
            return true;
        }
        if self.path.is_none() {
            cx.emit(EditorEvent::NeedsPath);
            return false;
        }
        if self.disk_changed {
            cx.emit(EditorEvent::SaveConflict);
            return false;
        }
        self.tidy_for_save(cx);
        let Some(path) = &self.path else { return false };
        // Deleted on disk with its folder: saving puts both back.
        if self.missing
            && let Some(parent) = path.parent()
        {
            std::fs::create_dir_all(parent).ok();
        }
        let text = self.buffer.to_string();
        let bytes = match crate::encoding::encode(&text, self.encoding) {
            Ok(bytes) => bytes,
            Err(c) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let encoding = self.encoding.label();
                cx.emit(EditorEvent::SaveFailed(format!(
                    "Couldn't save {name}: {encoding} has no “{c}”. Click {encoding} below to save it as UTF-8."
                )));
                return false;
            }
        };
        match crate::fs_ops::write_file(path, &bytes) {
            Ok(()) => {
                self.on_disk = Some(fingerprint(&text));
                self.missing = false;
                self.disk_changed = false;
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

    /// The text's size and the room between its lines, from the settings.
    pub fn set_text_size(&mut self, size: Pixels, spacing: f32, cx: &mut Context<Self>) {
        if size != self.font_size || spacing != self.line_spacing {
            self.font_size = size;
            self.line_spacing = spacing;
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
            .code_font(cx)
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
                    .code_font(cx)
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
            .children(blocks)
            .children(card.image.clone().map(|(path, about)| {
                // The image, at most this big, then its name and size, faintly.
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        gpui::img(path)
                            .max_w(px(320.))
                            .max_h(px(220.))
                            .object_fit(gpui::ObjectFit::ScaleDown)
                            .with_fallback(move || div().child(format!("Couldn't show {name}")).into_any_element()),
                    )
                    .child(div().text_size(px(ui::T_SM)).text_color(theme.faint).child(about))
            }));
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
                let byte = r.text_byte(crate::ui::index_at_x(&r.shaped, x - r.x));
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
        if let Some(line) = self.change_mark_at(event.position) {
            return self.review_change_at(line, cx);
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
        // A color's square: pick another in the Mac's color panel.
        if event.click_count == 1
            && !event.modifiers.modified()
            && let Some((start, written, color)) = self.swatch_at(event.position)
        {
            return self.pick_color(start, written, color, cx);
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
        // Pressed inside the selection: it may be dragged elsewhere (to move it, or with ⌥
        // to copy it). Nothing changes until it is.
        let plain = !(event.modifiers.shift || event.modifiers.secondary() || event.modifiers.control);
        let range = self.selection.range();
        if event.click_count == 1 && plain && self.extra.is_empty() && range.start < offset && offset < range.end {
            self.dragging =
                Some(DragUnit::Move { range, pressed: offset, version: self.buffer.version(), moved: false });
            self.drop_at = None;
            return cx.notify();
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
            DragUnit::Column(..) | DragUnit::Move { .. } => {}
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
            // ⌘-click on a task's box in Markdown ticks it.
            let (line, col) = self.buffer.point(offset);
            let text = self.buffer.line_text(line);
            let byte = text.char_indices().nth(col).map_or(text.len(), |(b, _)| b);
            if self.is_markdown()
                && !self.in_fence(line)
                && crate::markdown_view::task_box(&text).is_some_and(|b| b.contains(&byte) || b.end == byte)
                && let Some(ticked) = crate::markdown_view::toggle_task(&text)
            {
                let start = self.buffer.line_to_char(line);
                let caret = self.selection;
                self.edit(start..start + self.buffer.line_len(line), &ticked, EditKind::Other, cx);
                self.selection = caret;
                return;
            }
            match self.link_under(offset) {
                Some((_, target)) => self.follow_link(target, cx),
                None => self.go_to_definition_at(offset, cx),
            }
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
        let over_mark = self.change_mark_at(event.position).is_some() || self.swatch_at(event.position).is_some();
        if over_mark != self.over_change_mark {
            self.over_change_mark = over_mark;
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
                self.wrap.first_row(r.row.line) == row as usize,
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
            Some(DragUnit::Move { range, moved, .. }) => {
                // Over the text itself it wouldn't go anywhere; outside this editor's text (the
                // other side, the files) it isn't dropped at all.
                let within = range.start <= offset && offset <= range.end;
                let over_text = self.layout.as_ref().is_some_and(|l| l.text_bounds.contains(&position));
                self.drop_at = (!within && over_text).then_some(offset);
                let left = *moved || !within || !over_text;
                if let Some(DragUnit::Move { moved, .. }) = &mut self.dragging {
                    *moved = left;
                }
                return cx.notify();
            }
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

    /// Dragged text dropped at `at`: moved there (or copied, with ⌥), selected, as one step.
    fn drop_text(&mut self, range: Range<usize>, at: usize, copy: bool, cx: &mut Context<Self>) {
        let text = self.buffer.slice(range.clone());
        let len = text.chars().count();
        self.record_undo(EditKind::Other);
        let start = if copy {
            self.buffer.replace(at..at, &text);
            at
        } else if at > range.end {
            // Put down first, then taken from where it was (which is before it).
            self.buffer.replace(at..at, &text);
            self.buffer.replace(range.clone(), "");
            at - len
        } else {
            self.buffer.replace(range.clone(), "");
            self.buffer.replace(at..at, &text);
            at
        };
        self.single_cursor();
        self.selection = Selection { anchor: start, head: start + len };
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
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

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(DragUnit::Move { range, pressed, version, moved }) = self.dragging.take() {
            match self.drop_at.take() {
                // The text changed under the drag: what was picked up isn't there anymore.
                Some(_) if self.buffer.version() != version => cx.notify(),
                Some(at) => self.drop_text(range, at, event.modifiers.alt, cx),
                // A click in the selection without a drag: the caret goes there. Dragged and
                // brought back (or let go elsewhere): nothing changes.
                None if !moved => {
                    self.selection = Selection::caret(pressed);
                    self.touch(cx);
                    self.reveal_only = true;
                }
                None => cx.notify(),
            }
            return;
        }
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

    fn confirm_completion(&mut self, _: &ConfirmCompletion, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &self.fix_menu {
            return self.accept_fix(menu.selected, window, cx);
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

    fn toggle_block_comment_action(&mut self, _: &ToggleBlockComment, _: &mut Window, cx: &mut Context<Self>) {
        self.single_cursor();
        self.toggle_block_comment(cx);
    }

    fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        self.on_each_cursors_lines(cx, |this, cx| this.indent_lines(cx));
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        // In a Markdown table, Shift+Tab goes to the cell before.
        if !self.multi_cursor() && self.table_step(false, cx) {
            return;
        }
        self.on_each_cursors_lines(cx, |this, cx| this.outdent_lines(cx));
    }

    /// ⇥ or ⇧⇥ with the caret in a Markdown table: the table lined up and the caret in the
    /// next (or previous) cell, as one undo step. False out of a table.
    fn table_step(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        if !self.is_markdown() || !self.selection.is_empty() || self.multi_cursor() {
            return false;
        }
        let (line, column) = self.buffer.point(self.selection.head);
        // The table is around the caret: a window of lines is enough.
        let first = line.saturating_sub(500);
        let last = (line + 500).min(self.buffer.len_lines());
        let texts: Vec<String> = (first..last).map(|l| self.buffer.line_text(l)).collect();
        let lines: Vec<&str> = texts.iter().map(String::as_str).collect();
        let Some(step) = crate::markdown_view::table_step(&lines, line - first, column, forward) else { return false };
        let (start, end) = (first + step.lines.start, first + step.lines.end - 1);
        let range = self.buffer.line_to_char(start)..self.buffer.line_to_char(end) + self.buffer.line_len(end);
        self.edit(range, &step.new.join(self.style.line_ending.text()), EditKind::Other, cx);
        let (row, col) = step.caret;
        self.selection = Selection::caret(self.buffer.offset(start + row, col));
        self.touch(cx);
        true
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

    fn peek_definition(&mut self, _: &PeekDefinition, _: &mut Window, cx: &mut Context<Self>) {
        self.peek_definition_at(self.selection.head, cx);
    }

    fn expand_macro(&mut self, _: &ExpandMacro, _: &mut Window, cx: &mut Context<Self>) {
        self.expand_macro_at(self.selection.head, cx);
    }

    fn go_to_parent_module(&mut self, _: &GoToParentModule, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_target(crate::lsp_store::Target::ParentModule, cx);
    }

    fn open_cargo_toml(&mut self, _: &OpenCargoToml, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_target(crate::lsp_store::Target::CargoToml, cx);
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
    pub(crate) fn caret_bounds(&self, offset: usize) -> Option<Bounds<Pixels>> {
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
    /// The selection itself, pressed to be dragged elsewhere: its chars, where it was
    /// pressed (a click that doesn't drag puts the caret there), the text's version then (if
    /// the text changes under the drag, it's off), and whether it has left the selection.
    Move {
        range: Range<usize>,
        pressed: usize,
        version: u64,
        moved: bool,
    },
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
                    && (this.wrap_in_tag(c, cx) || this.type_smart_char(c, cx) || this.type_pair_char(c, cx))
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
            && (self.wrap_in_tag(c, cx) || self.type_smart_char(c, cx) || self.type_pair_char(c, cx))
        {
            return;
        }
        self.edit(range, text, EditKind::Typing, cx);
        match text {
            ">" => self.close_tag(cx),
            "/" => self.finish_closing_tag(cx),
            "}" | ")" | "]" => self.outdent_closer(cx),
            ":" if self.language_name() == "Python" => self.outdent_python_clause(cx),
            _ => {}
        }
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
        if let Some(preview) = &self.preview {
            return self.render_preview(preview, cx);
        }
        if self.reading {
            return self.render_reading(cx);
        }
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
            // In Markdown, files dropped from the Finder become links where they land.
            .when(self.is_markdown(), |text| {
                text.on_drop(cx.listener(|this, dropped: &gpui::ExternalPaths, window, cx| {
                    let at = this.offset_at(window.mouse_position());
                    cx.emit(EditorEvent::FilesDropped { paths: dropped.paths().to_vec(), at });
                }))
            })
            // A file dragged from the files: linked where it lands in Markdown, opened elsewhere.
            .on_drop(cx.listener(|this, dragged: &crate::file_tree::DraggedEntry, window, cx| {
                let path = dragged.path().to_path_buf();
                if this.is_markdown() {
                    let at = this.offset_at(window.mouse_position());
                    cx.emit(EditorEvent::FilesDropped { paths: vec![path], at });
                } else if path.is_file() {
                    cx.emit(EditorEvent::GoTo { path, range: Default::default() });
                }
            }))
            .size_full()
            .cursor(if self.link_word.is_some() || self.over_change_mark {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            })
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
            .on_action(cx.listener(Self::paste_as_is))
            .on_action(cx.listener(Self::toggle_task))
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
            .on_action(cx.listener(|this, _: &UseUtf8Encoding, _, cx| this.use_utf8(cx)))
            .on_action(cx.listener(Self::toggle_markdown_preview))
            .on_action(cx.listener(Self::keep_hunk))
            .on_action(cx.listener(Self::undo_hunk))
            .on_action(cx.listener(Self::fold))
            .on_action(cx.listener(Self::unfold))
            .on_action(cx.listener(Self::fold_all))
            .on_action(cx.listener(Self::unfold_all))
            .on_action(cx.listener(Self::go_to_definition))
            .on_action(cx.listener(Self::expand_macro))
            .on_action(cx.listener(Self::go_to_parent_module))
            .on_action(cx.listener(Self::open_cargo_toml))
            .on_action(cx.listener(Self::peek_definition))
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
            .on_action(cx.listener(Self::toggle_bookmark))
            .on_action(cx.listener(Self::next_bookmark))
            .on_action(cx.listener(Self::previous_bookmark))
            .on_action(cx.listener(Self::expand_selection))
            .on_action(cx.listener(Self::next_placeholder))
            .on_action(cx.listener(Self::previous_placeholder))
            .on_action(cx.listener(Self::end_snippet))
            .on_action(cx.listener(Self::shrink_selection))
            .on_action(cx.listener(Self::go_to_matching_bracket))
            .on_action(cx.listener(Self::newline_below))
            .on_action(cx.listener(Self::newline_above))
            .on_action(cx.listener(Self::use_selection_for_find))
            .on_action(cx.listener(Self::show_callers))
            .on_action(cx.listener(Self::join_lines))
            .on_action(cx.listener(Self::rewrap))
            .on_action(cx.listener(Self::delete_to_line_end))
            .on_action(cx.listener(Self::center_caret_line))
            .on_action(cx.listener(Self::move_subword_left))
            .on_action(cx.listener(Self::move_subword_right))
            .on_action(cx.listener(Self::select_subword_left))
            .on_action(cx.listener(Self::select_subword_right))
            .on_action(cx.listener(Self::delete_subword_left))
            .on_action(cx.listener(Self::jump_to_selection))
            .on_action(cx.listener(Self::yank))
            .on_action(cx.listener(Self::transpose))
            .on_action(cx.listener(Self::open_line))
            .on_action(cx.listener(Self::next_change))
            .on_action(cx.listener(Self::next_conflict))
            .on_action(cx.listener(Self::previous_conflict))
            .on_action(cx.listener(Self::previous_change))
            .on_action(cx.listener(Self::reverse_lines))
            .on_action(cx.listener(Self::remove_duplicate_lines))
            .on_action(cx.listener(Self::sort_lines))
            .on_action(cx.listener(Self::upper_case))
            .on_action(cx.listener(Self::lower_case))
            .on_action(cx.listener(Self::snake_case))
            .on_action(cx.listener(Self::camel_case))
            .on_action(cx.listener(Self::pascal_case))
            .on_action(cx.listener(Self::kebab_case))
            .on_action(cx.listener(Self::title_case))
            .on_action(cx.listener(Self::find_references))
            .on_action(cx.listener(Self::format_document))
            .on_action(cx.listener(Self::insert_table_of_contents))
            .on_action(cx.listener(Self::insert_footnote))
            .on_action(cx.listener(Self::format_selection))
            .on_action(cx.listener(Self::accept_ghost_word))
            .on_action(cx.listener(Self::accept_ghost_line))
            .on_action(cx.listener(Self::next_ghost))
            .on_action(cx.listener(Self::previous_ghost))
            .on_action(cx.listener(Self::dismiss_ghost_action))
            .on_action(cx.listener(Self::toggle_block_comment_action))
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
            .into_any_element()
    }
}

impl Editor {
    /// The find bar's search, while it's open.
    #[cfg(test)]
    pub fn find_query(&self, cx: &App) -> Option<crate::search::SearchQuery> {
        self.find_bar.as_ref().map(|bar| bar.read(cx).query(cx))
    }

    /// Whether the code font joins characters (`->`, `!=`) here: as the setting says, but
    /// never in prose, where a table's `|:-:|` or a note's `-->` would show as another symbol.
    pub fn ligatures(&self, cx: &App) -> bool {
        cx.global::<Settings>().ligatures && !self.is_prose()
    }

    /// The lines of the paragraph the caret is in, when the others fade (Settings, prose
    /// only): up to the blank lines around it.
    pub fn focused_paragraph(&self, cx: &App) -> Option<Range<usize>> {
        const FURTHEST: usize = 500;
        if !cx.global::<Settings>().dim_paragraphs || !self.is_prose() {
            return None;
        }
        let blank = |line: usize| self.buffer.line_text(line).trim().is_empty();
        let caret = self.caret_point().0;
        if blank(caret) {
            return Some(caret..caret + 1);
        }
        let mut start = caret;
        while start > 0 && caret - start < FURTHEST && !blank(start - 1) {
            start -= 1;
        }
        let mut end = caret + 1;
        while end < self.buffer.len_lines() && end - caret < FURTHEST && !blank(end) {
            end += 1;
        }
        Some(start..end)
    }

    /// Whether the gutter shows line numbers here, as Settings say.
    pub fn shows_line_numbers(&self, cx: &App) -> bool {
        match cx.global::<Settings>().line_numbers {
            crate::settings::LineNumbers::Shown => true,
            crate::settings::LineNumbers::InCode => !self.is_prose(),
            crate::settings::LineNumbers::Hidden => false,
        }
    }

    /// Prose: Markdown, or a text file (no language of its own). It wraps by its own setting.
    pub fn is_prose(&self) -> bool {
        if self.is_markdown() {
            return true;
        }
        let extension = self.path.as_ref().and_then(|p| p.extension()).and_then(|e| e.to_str());
        self.language().is_none() && matches!(extension, Some("txt" | "text" | "rst" | "adoc" | "org"))
    }

    /// Whether long lines wrap here: prose by its setting, code by the other.
    pub fn wraps(&self, cx: &App) -> bool {
        let settings = cx.global::<Settings>();
        if self.is_prose() { settings.wrap_prose } else { settings.word_wrap }
    }

    pub fn is_markdown(&self) -> bool {
        self.language().is_some_and(|l| l.name == "Markdown")
    }

    /// ⌘⇧V: Markdown as it reads, or back to its source.
    pub fn toggle_markdown_preview(&mut self, _: &ToggleMarkdownPreview, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_markdown() && !self.reading {
            let at = self.selection.head;
            return self.show_notice(at, "The preview is for Markdown files.".into(), cx);
        }
        self.reading = !self.reading;
        self.close_completion(cx);
        self.close_hover(cx);
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// The source's first line on screen, for a preview beside it to follow.
    pub fn top_line(&self) -> usize {
        let row = (self.scroll.target_y / f32::from(self.line_height())).max(0.) as usize;
        self.wrap.line_of_row(row)
    }

    /// In the preview: scrolls to the block the source shows at its top, when that changes.
    pub fn follow_source_line(&mut self, line: usize, cx: &mut Context<Self>) {
        if !self.reading || self.followed_line == Some(line) {
            return;
        }
        self.followed_line = Some(line);
        let Some((_, blocks)) = &self.markdown else { return };
        let ix = blocks.iter().rposition(|(start, _)| *start <= line).unwrap_or(0);
        if line == 0 {
            self.reading_scroll.set_offset(gpui::point(px(0.), px(0.)));
        } else {
            self.reading_scroll.scroll_to_top_of_item(ix);
        }
        cx.notify();
    }

    /// In the preview: to the heading whose anchor is `anchor` (a `#section` link).
    fn scroll_preview_to(&mut self, anchor: &str, cx: &mut Context<Self>) {
        let Some((_, blocks)) = &self.markdown else { return };
        // Numbered as anchors are: a second "Install" is #install-1.
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let found = blocks.iter().position(|(_, block)| {
            let crate::markdown_view::Block::Heading(_, text) = block else { return false };
            let base = crate::markdown_view::slug(&crate::markdown_view::plain_text(text));
            let n = seen.entry(base.clone()).or_insert(0);
            let numbered = if *n == 0 { base } else { format!("{base}-{n}") };
            *n += 1;
            numbered == anchor
        });
        if let Some(ix) = found {
            self.reading_scroll.scroll_to_top_of_item(ix);
            cx.notify();
        }
    }

    /// The preview: the document drawn as it reads, in a column, scrolling.
    fn render_reading(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let revision = self.buffer.revision();
        let blocks = match &self.markdown {
            Some((r, blocks)) if *r == revision => blocks.clone(),
            _ => {
                let blocks = Rc::new(crate::markdown_view::parse_located(&self.buffer.to_string()));
                self.markdown = Some((revision, blocks.clone()));
                blocks
            }
        };
        let theme = cx.global::<Theme>().clone();
        let fonts = cx.global::<Fonts>();
        let this = cx.entity().downgrade();
        // How many boxes are drawn, for a click to check it finds as many in the text.
        let drawn = Rc::new(std::cell::Cell::new(0));
        let tick = {
            let (this, drawn) = (this.clone(), drawn.clone());
            Rc::new(move |task: usize, _: &mut Window, cx: &mut App| {
                this.update(cx, |editor, cx| editor.tick_task(task, drawn.get(), cx)).ok();
            }) as crate::markdown_view::Ticker
        };
        let style = crate::markdown_view::Style {
            theme: theme.clone(),
            ui_font: fonts.ui.clone(),
            code_font: fonts.code.clone(),
            code_features: crate::fonts::code_features(cx.global::<Settings>().ligatures),
            base: self.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_default(),
            open: Rc::new(move |target, _, cx| match target {
                crate::markdown_view::Follow::Web(url) => cx.open_url(&url),
                crate::markdown_view::Follow::File(path) => {
                    this.update(cx, |_, cx| cx.emit(EditorEvent::GoTo { path, range: Default::default() })).ok();
                }
                crate::markdown_view::Follow::Heading(anchor) => {
                    this.update(cx, |editor, cx| editor.scroll_preview_to(&anchor, cx)).ok();
                }
            }),
            tick: Some(tick),
            tasks: Default::default(),
        };
        let mut context = KeyContext::new_with_defaults();
        context.add("Editor");
        context.add("MarkdownPreview");
        div()
            .key_context(context)
            .track_focus(&self.focus_handle)
            .size_full()
            .on_action(cx.listener(Self::toggle_markdown_preview))
            .on_action(cx.listener(Self::save))
            .child(
                // Each block a child of the scroller, so the preview can scroll to one.
                div()
                    .id("markdown-preview")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.reading_scroll)
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .py(px(28.))
                    .font_family(style.ui_font.clone())
                    .text_size(px(15.))
                    .line_height(px(24.))
                    .text_color(theme.foreground)
                    .children(
                        {
                            let rendered = crate::markdown_view::render_blocks(&blocks, &style);
                            drawn.set(style.tasks.get());
                            rendered
                        }
                        .into_iter()
                        .map(|block| {
                            div()
                                .w_full()
                                .flex_none()
                                .flex()
                                .justify_center()
                                .px(px(32.))
                                .child(div().w_full().max_w(px(760.)).flex().flex_col().child(block))
                        }),
                    ),
            )
            .into_any_element()
    }

    /// An image at its own size (smaller if it doesn't fit), or a line saying the file
    /// isn't text.
    fn render_preview(&self, preview: &crate::preview::Preview, cx: &Context<Self>) -> AnyElement {
        use gpui::{ObjectFit, StyledImage, img};
        let theme = cx.global::<Theme>();
        let (muted, faint) = (theme.muted, theme.faint);
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let note = move |text: String| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .text_size(px(crate::ui::T_MD))
                .text_color(muted)
                .child(text)
        };
        let content = match (preview, &self.path) {
            (crate::preview::Preview::Image { .. }, Some(path)) => {
                let name = name.clone().unwrap_or_default();
                img(path.clone())
                    .size_full()
                    .object_fit(ObjectFit::ScaleDown)
                    .with_fallback(move || note(format!("Couldn't show {name}")).into_any_element())
                    .into_any_element()
            }
            (crate::preview::Preview::Unreadable { .. }, _) => {
                note(format!("Couldn't read {}", name.unwrap_or_else(|| "this file".into())))
                    .child(div().text_size(px(crate::ui::T_SM)).text_color(faint).child(preview.summary()))
                    .into_any_element()
            }
            _ => note(format!("{} isn't text", name.unwrap_or_else(|| "This file".into())))
                .child(div().text_size(px(crate::ui::T_SM)).text_color(faint).child(preview.summary()))
                .into_any_element(),
        };
        div()
            .key_context("Preview")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(32.))
            .font_family(cx.global::<Fonts>().ui.clone())
            .child(content)
            .into_any_element()
    }
}

/// Prose files bigger than this don't show their word count.
const WORD_COUNT_BYTES: usize = 1 << 20;

/// Words in `text`: runs of non-space with a letter or digit in them (so `#`, `-` and `|`
/// in Markdown don't count). `n` and `in_word` carry the count over from the text before.
fn count_words(text: &str, mut n: usize, mut in_word: bool) -> (usize, bool) {
    for c in text.chars() {
        if c.is_whitespace() {
            n += usize::from(in_word);
            in_word = false;
        } else if c.is_alphanumeric() {
            in_word = true;
        }
    }
    (n, in_word)
}

/// The words in `text`.
pub fn words_in(text: &str) -> usize {
    let (n, in_word) = count_words(text, 0, false);
    n + usize::from(in_word)
}

/// `[words](address)` when `selected` is some words on one line and `pasted` a single web
/// address (and the words aren't an address themselves). ⌥⇧⌘V pastes the address as it is.
fn markdown_link(selected: &str, pasted: &str) -> Option<String> {
    let is_address =
        |t: &str| (t.starts_with("https://") || t.starts_with("http://")) && !t.contains(char::is_whitespace);
    let words = selected.trim();
    (is_address(pasted) && !words.is_empty() && !words.contains('\n') && !is_address(words)).then(|| {
        // Spaces around the words stay outside the link.
        let before = &selected[..selected.len() - selected.trim_start().len()];
        let after = &selected[selected.trim_end().len()..];
        format!("{before}[{words}]({pasted}){after}")
    })
}

/// A moment as `2026-10-06-120312`, in UTC (seconds since 1970).
fn utc_stamp(secs: u64) -> String {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970 to a date (Howard Hinnant's civil_from_days).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}-{:02}{:02}{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    /// A box clicked in the preview ticks its line; with the text reading differently from
    /// what was drawn, nothing changes.
    #[gpui::test]
    fn ticking_a_task_in_the_preview(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = "# Shop\n\n- [ ] milk\n- [ ] eggs\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("todo.md")), cx));
        e.update(cx, |e, cx| {
            e.tick_task(1, 2, cx);
            assert_eq!(e.buffer.to_string(), "# Shop\n\n- [ ] milk\n- [x] eggs\n");
            e.tick_task(1, 2, cx);
            assert_eq!(e.buffer.to_string(), text);
            e.tick_task(0, 3, cx);
            assert_eq!(e.buffer.to_string(), text);
        });
        // A real click on the second box, in the preview.
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle);
            e.toggle_markdown_preview(&ToggleMarkdownPreview, window, cx);
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("task 1").expect("the second box is drawn");
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "# Shop\n\n- [ ] milk\n- [x] eggs\n"));
    }

    /// ⇧⌘X ticks the caret's task, makes selected lines tasks (all one way), and ⌘-click on
    /// a box in the text ticks it.
    #[gpui::test]
    fn ticking_tasks_in_the_text(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let text = "# Shop\n\n- [ ] milk\n- eggs\nTea\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("todo.md")), cx));
        e.update_in(cx, |e, window, _| {
            window.focus(&e.focus_handle);
            e.selection = Selection::caret(e.buffer.offset(2, 8));
        });
        cx.simulate_keystrokes("cmd-shift-x");
        let now = |cx: &mut gpui::VisualTestContext| e.read_with(cx, |e, _| e.buffer.to_string());
        assert_eq!(now(cx), "# Shop\n\n- [x] milk\n- eggs\nTea\n");
        e.update(cx, |e, _| assert_eq!(e.caret_point(), (2, 8), "the caret stays by its text"));
        // The three lines: the first was ticked, so it unticks and the others become tasks.
        e.update(cx, |e, _| e.selection = Selection { anchor: e.buffer.offset(2, 0), head: e.buffer.offset(5, 0) });
        cx.simulate_keystrokes("cmd-shift-x");
        assert_eq!(now(cx), "# Shop\n\n- [ ] milk\n- [ ] eggs\n- [ ] Tea\n");
        // ⌘-click on the second box.
        cx.run_until_parked();
        let at = e.read_with(cx, |e, _| {
            let layout = e.layout.as_ref().unwrap();
            let r = layout.rows.iter().find(|r| r.row.line == 3).unwrap();
            let x = layout.text_origin.x + r.x + r.shaped.x_for_index(r.shown_byte(3)) + gpui::px(2.);
            gpui::point(x, layout.text_origin.y + layout.line_height * 3.5)
        });
        cx.simulate_click(at, gpui::Modifiers::command());
        assert_eq!(now(cx), "# Shop\n\n- [ ] milk\n- [x] eggs\n- [ ] Tea\n");
    }

    /// Insert Footnote: the next number's mark at the caret, its note at the end (with the
    /// notes there), the caret there; one undo takes both back.
    #[gpui::test]
    fn footnotes_are_marked_here_and_written_at_the_end(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = "# Essay\n\nA claim. Another.\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("a.md")), cx));
        e.update_in(cx, |e, window, cx| {
            e.selection = Selection::caret(e.buffer.offset(2, 8));
            e.insert_footnote(&InsertFootnote, window, cx);
            assert_eq!(e.buffer.to_string(), "# Essay\n\nA claim.[^1] Another.\n\n[^1]: \n");
            assert_eq!(e.caret_point(), (4, 6), "ready to write the note");
            e.replace_text_in_range(None, "Source.", window, cx);
            e.selection = Selection::caret(e.buffer.offset(2, 21));
            e.insert_footnote(&InsertFootnote, window, cx);
            assert_eq!(
                e.buffer.to_string(),
                "# Essay\n\nA claim.[^1] Another.[^2]\n\n[^1]: Source.\n[^2]: \n",
                "the next number, with the notes"
            );
            e.step_history(true, cx);
            assert_eq!(e.buffer.to_string(), "# Essay\n\nA claim.[^1] Another.\n\n[^1]: Source.\n");
        });
        assert_eq!(crate::markdown_view::next_footnote("x[^note] y[^7] z[^2]"), 8);
        // At the very end of the text (where a footnote often goes), with or without a last
        // line break.
        for (text, expected) in
            [("Hello world\n", "Hello world[^1]\n\n[^1]: \n"), ("Hello world", "Hello world[^1]\n\n[^1]: ")]
        {
            let (e, cx) =
                cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("b.md")), cx));
            e.update_in(cx, |e, window, cx| {
                e.selection = Selection::caret(e.buffer.offset(0, 11));
                e.insert_footnote(&InsertFootnote, window, cx);
                assert_eq!(e.buffer.to_string(), expected);
                assert_eq!(e.caret_point(), (2, 6));
            });
        }
    }

    /// ⌥⇧F in Markdown (no server for it) lines the tables up, as one undo step.
    #[gpui::test]
    fn format_lines_markdown_tables_up(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let text = "|a|bb|\n|-|-|\n|ccc|d|\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("t.md")), cx));
        e.update_in(cx, |e, window, _| window.focus(&e.focus_handle));
        cx.simulate_keystrokes("alt-shift-f");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "| a   | bb  |\n| --- | --- |\n| ccc | d   |\n"));
        cx.simulate_keystrokes("cmd-z");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), text));
    }

    /// ⇥ in a Markdown table goes cell to cell, lining it up, and makes a row at the end;
    /// ⇧⇥ goes back.
    #[gpui::test]
    fn tab_moves_through_a_markdown_table(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let text = "|Item|Qty|\n|-|-|\n|milk|2|\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("t.md")), cx));
        e.update_in(cx, |e, window, _| {
            window.focus(&e.focus_handle);
            e.selection = Selection::caret(2);
        });
        let caret = |cx: &mut gpui::VisualTestContext| e.read_with(cx, |e, _| e.caret_point());
        cx.simulate_keystrokes("tab");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "| Item | Qty |\n| ---- | --- |\n| milk | 2   |\n"));
        assert_eq!(caret(cx), (0, 9));
        cx.simulate_keystrokes("tab tab tab");
        assert_eq!(caret(cx), (3, 2));
        cx.simulate_input("eggs");
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(caret(cx), (2, 9));
        e.update(cx, |e, _| assert_eq!(e.buffer.line_text(3), "| eggs |     |"));
    }

    /// ⌥⌘/ wraps the selection in /* */ and back; over lines, the lines; Python has none.
    #[gpui::test]
    fn block_comments_wrap_and_unwrap(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let text = "let a = b + c;\nlet d = 1;\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("x.rs")), cx));
        e.update_in(cx, |e, window, _| {
            window.focus(&e.focus_handle);
            e.selection = Selection { anchor: 8, head: 13 };
        });
        cx.simulate_keystrokes("alt-cmd-/");
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.line_text(0), "let a = /* b + c */;");
            assert_eq!(e.buffer.slice(e.selection.range()), "/* b + c */");
        });
        cx.simulate_keystrokes("alt-cmd-/");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), text));
        // Two whole lines.
        e.update(cx, |e, _| e.selection = Selection { anchor: 0, head: 24 });
        cx.simulate_keystrokes("alt-cmd-/");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "/* let a = b + c;\nlet d = 1; */\n"));
        // ⌘/ still uses line comments.
        cx.simulate_keystrokes("cmd-z cmd-/");
        e.update(cx, |e, _| assert!(e.buffer.to_string().starts_with("// let a")));
    }

    #[test]
    fn words_are_counted_as_people_count_them() {
        assert_eq!(words_in("# A title\n\n- one two | three\n"), 5);
        assert_eq!(words_in("l'été, c'est ça."), 3);
        assert_eq!(words_in("   "), 0);
        // Across the rope's pieces, a word cut in two is one.
        let (n, carried) = count_words("hel", 0, false);
        assert_eq!(count_words("lo world", n, carried), (1, true));
    }

    /// ⌘E: the word at the caret is searched for; ⌘G goes to its next use.
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn the_selection_becomes_the_search(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let text = "let total = price * qty;\nprint(total);\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("x.py")), cx));
        e.update_in(cx, |e, window, _| {
            window.focus(&e.focus_handle);
            e.selection = Selection::caret(6);
        });
        cx.simulate_keystrokes("cmd-e cmd-g");
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.slice(e.selection.range()), "total");
            assert_eq!(e.selection.range(), 31..36);
        });
    }

    /// A search made in one file is what ⌘G looks for in another.
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn searches_carry_from_file_to_file(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let open = |cx: &mut TestAppContext, name: &str, text: &str| {
            let (name, text) = (PathBuf::from(name), text.to_string());
            let (e, vcx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(name), cx));
            e.update_in(vcx, |e, window, _| window.focus(&e.focus_handle));
            (e, vcx.window_handle())
        };
        let (first, first_window) = open(cx, "a.py", "total = 1\n");
        let (second, second_window) = open(cx, "b.py", "x = 2\nprint(total)\n");
        let mut first_cx = gpui::VisualTestContext::from_window(first_window, cx);
        first.update(&mut first_cx, |e, _| e.selection = Selection::caret(2));
        first_cx.simulate_keystrokes("cmd-e");
        let mut second_cx = gpui::VisualTestContext::from_window(second_window, cx);
        second_cx.simulate_keystrokes("cmd-g");
        second.update(&mut second_cx, |e, _| assert_eq!(e.buffer.slice(e.selection.range()), "total"));
    }

    /// A click on a changed line's mark in the gutter opens the review of the file's
    /// changes, at that one; Esc takes it back.
    #[gpui::test]
    fn clicking_a_change_mark_opens_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let base = "one\ntwo\nthree\nfour\n";
        let text = "one\ntwo\nTHREE\nfour\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("x.txt")), cx));
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle);
            e.git_base = Some(base.into());
            e.refresh_git_hunks(std::time::Duration::ZERO, cx);
        });
        cx.run_until_parked();
        let mark = e.read_with(cx, |e, _| {
            let layout = e.layout.as_ref().unwrap();
            gpui::point(layout.text_bounds.left() - px(6.), layout.text_origin.y + layout.line_height * 2.5)
        });
        // Over the mark: a hand. On an unchanged line's place: nothing to open.
        let unchanged = gpui::point(mark.x, mark.y - e.read_with(cx, |e, _| e.line_height()) * 2.);
        e.read_with(cx, |e, _| assert_eq!(e.change_mark_at(unchanged), None));
        cx.simulate_click(mark, gpui::Modifiers::none());
        e.update(cx, |e, _| {
            assert!(e.in_review());
            assert_eq!(e.caret_point().0, 2);
        });
        cx.simulate_keystrokes("escape");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), base));
    }

    #[test]
    fn an_address_pasted_over_words_links_them() {
        let url = "https://example.com/a";
        assert_eq!(markdown_link("the docs", url).as_deref(), Some("[the docs](https://example.com/a)"));
        assert_eq!(markdown_link(" the docs ", url).as_deref(), Some(" [the docs](https://example.com/a) "));
        assert_eq!(markdown_link("", url), None);
        assert_eq!(markdown_link("two\nlines", url), None);
        assert_eq!(markdown_link("http://old.example", url), None);
        assert_eq!(markdown_link("words", "not an address"), None);
        assert_eq!(markdown_link("words", "https://a.b c"), None);
    }

    #[gpui::test]
    fn no_ligatures_in_prose(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        for (name, joined) in [("main.rs", true), ("notes.md", false), ("todo.txt", false)] {
            let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(""), Some(PathBuf::from(name)), cx));
            e.read_with(cx, |e, cx| assert_eq!(e.ligatures(cx), joined, "{name}"));
        }
    }

    #[test]
    fn stamps_are_dates_and_times() {
        assert_eq!(utc_stamp(0), "1970-01-01-000000");
        assert_eq!(utc_stamp(951_782_400 + 3_723), "2000-02-29-010203");
        assert_eq!(utc_stamp(1_791_201_600), "2026-10-05-120000");
    }

    /// An image pasted in Markdown is saved next to the file and linked where it goes.
    #[gpui::test]
    fn an_image_pasted_in_markdown_is_saved_and_linked(cx: &mut TestAppContext) {
        let dir = crate::tools::test_dir("paste-image");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let path = dir.join("notes.md");
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text("See: \n"), Some(path), cx));
        e.update_in(cx, |e, window, _| {
            window.focus(&e.focus_handle);
            e.selection = Selection::caret(5);
        });
        let png = vec![0x89, b'P', b'N', b'G'];
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            png.clone(),
        )));
        cx.simulate_keystrokes("cmd-v");
        let text = e.read_with(cx, |e, _| e.buffer.to_string());
        let name = text.strip_prefix("See: ![").and_then(|rest| rest.split(']').next()).unwrap_or_default().to_string();
        assert!(name.starts_with("pasted-"), "{text}");
        assert_eq!(text, format!("See: ![{name}]({name}.png)\n"));
        assert_eq!(std::fs::read(dir.join(format!("{name}.png"))).unwrap(), png);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Enter in a Markdown list starts the next item; on an empty one, it ends the list.
    #[gpui::test]
    fn enter_carries_markdown_lists_on(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(""), Some(PathBuf::from("todo.md")), cx));
        e.update_in(cx, |e, window, _| window.focus(&e.focus_handle));
        cx.simulate_input("1. milk");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("eggs");
        cx.simulate_keystrokes("enter enter");
        cx.simulate_input("done");
        cx.run_until_parked();
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "1. milk\n2. eggs\ndone");
        // Tab in an item nests it, the caret where it was in the text; ⇧Tab brings it back.
        e.update(cx, |e, cx| e.set_caret_point((1, 5), cx));
        cx.simulate_keystrokes("tab");
        assert_eq!(e.read_with(cx, |e, _| (e.buffer.line_text(1), e.caret_point())), ("    2. eggs".into(), (1, 9)));
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.line_text(1)), "2. eggs");
        // Not in a code block.
        let (code, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text("```\n- item"), Some(PathBuf::from("x.md")), cx));
        code.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle);
            e.set_caret_point((1, 6), cx);
        });
        cx.simulate_keystrokes("enter");
        assert_eq!(code.read_with(cx, |e, _| e.buffer.to_string()), "```\n- item\n");
    }

    #[gpui::test]
    fn a_file_deleted_with_its_folder_is_noticed_and_saved_back(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let dir = crate::tools::test_dir("missing");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sub/a.rs");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::open(path.clone(), None, cx));
        editor.update(cx, |e, cx| {
            assert!(!e.check_missing(cx));
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
            // Reloading keeps the text, and notes the file is gone.
            e.reload_from_disk(cx);
            assert!(e.missing);
            assert_eq!(e.buffer.to_string(), "fn a() {}\n");
            assert!(e.save_to_disk(cx));
            assert!(!e.missing);
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn a() {}\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The watcher reporting Null's own save after typing went on: not a change on disk,
    /// so saving (and auto-saving) goes on without asking.
    #[gpui::test]
    fn its_own_save_isnt_a_change_on_disk(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let dir = crate::tools::test_dir("own-save");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        std::fs::write(&path, "one\n").unwrap();
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::open(path.clone(), None, cx));
        editor.update(cx, |e, cx| {
            e.restore_unsaved("two\n", cx);
            assert!(e.save_to_disk(cx));
            e.restore_unsaved("three\n", cx);
            e.reload_from_disk(cx);
            assert!(!e.disk_changed, "its own save");
            // Someone else's change still counts.
            std::fs::write(&path, "theirs\n").unwrap();
            e.reload_from_disk(cx);
            assert!(e.disk_changed);
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Insert Table of Contents: at the caret first, then brought up to date in place; its
    /// links lead somewhere.
    #[gpui::test]
    fn a_table_of_contents_is_inserted_then_kept_up_to_date(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let dir = crate::tools::test_dir("toc");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("guide.md");
        std::fs::write(&path, "# Guide\n\n\n## Install\n## Use\n").unwrap();
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::open(path.clone(), None, cx));
        editor.update_in(cx, |e, window, cx| {
            e.selection = Selection::caret(e.buffer.offset(2, 0));
            e.insert_table_of_contents(&InsertTableOfContents, window, cx);
            assert_eq!(
                e.buffer.to_string(),
                "# Guide\n\n<!-- toc -->\n- [Install](#install)\n- [Use](#use)\n<!-- /toc -->\n\n## Install\n## Use\n"
            );
            for line in 0..e.buffer.len_lines() {
                assert!(e.broken_links_on_line(line, &e.buffer.line_text(line)).is_empty(), "line {line}");
            }
            // A new heading, and again from the end: the same list, up to date.
            let end = e.buffer.len_chars();
            e.edit(end..end, "## Help\n", EditKind::Other, cx);
            e.selection = Selection::caret(e.buffer.len_chars());
            e.insert_table_of_contents(&InsertTableOfContents, window, cx);
            let text = e.buffer.to_string();
            assert_eq!(text.matches("<!-- toc -->").count(), 1);
            assert!(text.contains("- [Use](#use)\n- [Help](#help)\n<!-- /toc -->"), "{text}");
            // Another tool's start mark, with no end of Null's after it: left alone.
            let start = e.buffer.len_chars();
            e.edit(start..start, "<!-- toc -->\nkeep me\n", EditKind::Other, cx);
            e.insert_table_of_contents(&InsertTableOfContents, window, cx);
            assert!(e.buffer.to_string().ends_with("<!-- toc -->\nkeep me\n"));
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Spreadsheet cells pasted in Markdown: a table on lines of its own. ⌥⇧⌘V, or a code
    /// file, pastes them as they are.
    #[gpui::test]
    fn spreadsheet_cells_paste_as_a_table(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let cells = "Item\tQty\nTea\t2\n";
        let (editor, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text("Prices:"), Some(PathBuf::from("notes.md")), cx));
        editor.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle);
            e.selection = Selection::caret(7);
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(cells.to_string()));
        });
        cx.simulate_keystrokes("cmd-v");
        editor.read_with(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "Prices:\n\n| Item | Qty |\n| ---- | --- |\n| Tea  | 2   |\n")
        });
        cx.simulate_keystrokes("alt-shift-cmd-v");
        editor.read_with(cx, |e, _| assert!(e.buffer.to_string().ends_with("| Tea  | 2   |\nItem\tQty\nTea\t2\n")));
    }

    /// Dimming the other paragraphs: the caret's paragraph is up to the blank lines around
    /// it; only in prose, only when asked.
    #[gpui::test]
    fn the_paragraph_being_written(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings { dim_paragraphs: true, ..Settings::default() });
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = "one\ntwo\n\nthree\nfour\n\nfive\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some("notes.md".into()), cx));
        e.update(cx, |e, cx| {
            e.set_caret_point((4, 1), cx);
            assert_eq!(e.focused_paragraph(cx), Some(3..5));
            e.set_caret_point((2, 0), cx);
            assert_eq!(e.focused_paragraph(cx), Some(2..3), "a blank line alone");
        });
        let (code, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some("a.rs".into()), cx));
        code.read_with(cx, |e, cx| assert_eq!(e.focused_paragraph(cx), None, "not in code"));
    }

    /// Typewriter scrolling: after moving by keyboard, the caret's line sits mid-window.
    #[gpui::test]
    fn typewriter_keeps_the_line_in_the_middle(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings { typewriter: true, ..Settings::default() });
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some("notes.txt".into()), cx));
        cx.run_until_parked();
        e.update(cx, |e, cx| e.go_to_line(100, cx));
        // Let the scroll glide to where it's going.
        for _ in 0..60 {
            cx.executor().advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            e.update(cx, |_, cx| cx.notify());
        }
        e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().expect("drawn");
            let caret = e.caret_bounds(e.selection.head).expect("on screen").center().y;
            let middle = l.text_bounds.center().y;
            assert!((caret - middle).abs() < l.line_height, "caret {caret:?}, middle {middle:?}");
        });
    }

    /// Line numbers: in code only leaves them out of Markdown; hidden, out of code too.
    #[gpui::test]
    fn line_numbers_show_where_settings_say(cx: &mut TestAppContext) {
        use crate::settings::LineNumbers;
        cx.update(|cx| {
            cx.set_global(Settings { line_numbers: LineNumbers::InCode, ..Settings::default() });
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text_left = |name: &str, cx: &mut TestAppContext| {
            let name = PathBuf::from(name);
            let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text("one\ntwo\n"), Some(name), cx));
            cx.run_until_parked();
            e.read_with(cx, |e, _| {
                let l = e.layout.as_ref().expect("drawn");
                l.text_bounds.left() - l.bounds.left()
            })
        };
        let (prose, code) = (text_left("notes.md", cx), text_left("a.rs", cx));
        assert!(prose < code, "no numbers in Markdown: {prose:?} vs {code:?}");
        cx.update(|cx| cx.set_global(Settings { line_numbers: LineNumbers::Hidden, ..Settings::default() }));
        assert_eq!(text_left("a.rs", cx), prose, "hidden in code too");
    }

    /// Dragging the selection moves it where it's dropped; with ⌥, copies it. A click in it
    /// without a drag just puts the caret there.
    #[gpui::test]
    fn the_selection_drags_elsewhere(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text("one two three\n"), Some("x.txt".into()), cx));
        e.update_in(cx, |e, window, cx| window.focus(&e.focus_handle(cx)));
        let drag = |cx: &mut gpui::VisualTestContext, from: usize, to: usize, alt: bool| {
            let at = |cx: &mut gpui::VisualTestContext, offset| {
                e.read_with(cx, |e, _| e.caret_bounds(offset).expect("drawn").center())
            };
            let modifiers = gpui::Modifiers { alt, ..Default::default() };
            let (from, to) = (at(cx, from), at(cx, to));
            cx.simulate_event(gpui::MouseDownEvent {
                position: from,
                button: gpui::MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            });
            if from != to {
                cx.simulate_event(gpui::MouseMoveEvent {
                    position: to,
                    pressed_button: Some(gpui::MouseButton::Left),
                    modifiers,
                });
            }
            cx.simulate_event(gpui::MouseUpEvent {
                position: to,
                button: gpui::MouseButton::Left,
                modifiers,
                click_count: 1,
            });
            cx.run_until_parked();
        };
        let select = |cx: &mut gpui::VisualTestContext, range: Range<usize>| {
            e.update(cx, |e, cx| {
                e.selection = Selection { anchor: range.start, head: range.end };
                cx.notify();
            });
            cx.run_until_parked();
        };
        // "two" dragged to the end.
        select(cx, 4..7);
        drag(cx, 5, 13, false);
        e.read_with(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "one  threetwo\n");
            assert_eq!(e.selection.range(), 10..13);
        });
        // ⌥: "one" copied to the start of "three".
        select(cx, 0..3);
        drag(cx, 1, 5, true);
        e.read_with(cx, |e, _| assert_eq!(e.buffer.to_string(), "one  onethreetwo\n"));
        // Let go outside the text (over the gutter): nothing moves, the selection stays.
        select(cx, 0..3);
        let gutter = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().expect("drawn");
            gpui::point(l.bounds.left() + gpui::px(4.), l.text_origin.y + l.line_height / 2.)
        });
        let inside = e.read_with(cx, |e, _| e.caret_bounds(1).expect("drawn").center());
        cx.simulate_event(gpui::MouseDownEvent {
            position: inside,
            button: gpui::MouseButton::Left,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(gpui::MouseMoveEvent {
            position: gutter,
            pressed_button: Some(gpui::MouseButton::Left),
            modifiers: Default::default(),
        });
        cx.simulate_event(gpui::MouseUpEvent {
            position: gutter,
            button: gpui::MouseButton::Left,
            modifiers: Default::default(),
            click_count: 1,
        });
        cx.run_until_parked();
        e.read_with(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "one  onethreetwo\n");
            assert_eq!(e.selection.range(), 0..3);
        });
        // A click inside without dragging: just the caret.
        select(cx, 0..3);
        drag(cx, 2, 2, false);
        e.read_with(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "one  onethreetwo\n");
            assert_eq!(e.selection.range(), 2..2);
        });
    }

    /// Markdown: * _ ~ over a selection wrap it, and it stays selected to wrap again
    /// (**bold**). In code, a * over a selection replaces it as ever.
    #[gpui::test]
    fn markdown_marks_wrap_the_selection(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let dir = crate::tools::test_dir("markdown-marks");
        std::fs::create_dir_all(&dir).unwrap();
        let notes = dir.join("notes.md");
        std::fs::write(&notes, "a big word\n").unwrap();
        let code = dir.join("a.rs");
        std::fs::write(&code, "a * b\n").unwrap();
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::open(notes.clone(), None, cx));
        editor.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle);
            e.selection = Selection { anchor: 2, head: 5 };
            cx.notify();
        });
        cx.simulate_input("*");
        cx.simulate_input("*");
        editor.read_with(cx, |e, _| assert_eq!(e.buffer.to_string(), "a **big** word\n"));
        editor.update(cx, |e, cx| {
            e.selection = Selection { anchor: 10, head: 14 };
            cx.notify();
        });
        cx.simulate_input("_");
        editor.read_with(cx, |e, _| assert_eq!(e.buffer.to_string(), "a **big** _word_\n"));

        let (editor, cx) = cx.add_window_view(|_, cx| Editor::open(code.clone(), None, cx));
        editor.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle);
            e.selection = Selection { anchor: 0, head: 1 };
            cx.notify();
        });
        cx.simulate_input("*");
        editor.read_with(cx, |e, _| assert_eq!(e.buffer.to_string(), "* * b\n"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Misspelled words are marked in prose and comments, never in code; ⌘. on one offers
    /// corrections and ↵ takes the first.
    #[gpui::test]
    fn misspelled_words_are_marked_and_corrected(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let dir = crate::tools::test_dir("spelling");
        std::fs::create_dir_all(&dir).unwrap();
        let notes = dir.join("notes.md");
        std::fs::write(&notes, "I saw teh cat `teh` there.\n```\nteh\n```\n").unwrap();
        let code = dir.join("a.rs");
        std::fs::write(&code, "// teh end\nfn teh() {}\n").unwrap();
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::open(notes.clone(), None, cx));
        cx.run_until_parked();
        editor.update_in(cx, |e, window, cx| {
            let marked = |e: &Editor, line: usize, cx: &App| {
                let text = e.buffer.line_text(line);
                e.misspellings_on_line(line, &text, cx).into_iter().map(|r| text[r].to_string()).collect::<Vec<_>>()
            };
            assert_eq!(marked(e, 0, cx), ["teh"], "in prose, not in ticks");
            assert!(marked(e, 2, cx).is_empty(), "not in a fence");
            window.focus(&e.focus_handle);
            e.selection = Selection::caret(7);
        });
        cx.simulate_keystrokes("cmd-. enter");
        cx.run_until_parked();
        editor.read_with(cx, |e, _| assert!(e.buffer.to_string().starts_with("I saw the cat `teh`")));

        let (editor, cx) = cx.add_window_view(|_, cx| Editor::open(code.clone(), None, cx));
        cx.run_until_parked();
        editor.update(cx, |e, cx| {
            let marked = |line: usize| {
                let text = e.buffer.line_text(line);
                e.misspellings_on_line(line, &text, cx).into_iter().map(|r| text[r].to_string()).collect::<Vec<_>>()
            };
            assert_eq!(marked(0), ["teh"], "in a comment");
            assert!(marked(1).is_empty(), "not in code");
        });
        std::fs::remove_dir_all(&dir).ok();
    }
}
