use crate::buffer::Buffer;
use crate::element::EditorElement;
use crate::highlight::{Highlighter, Span};
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, ShapedLine, Task, UTF16Selection, Window, actions, div, point, prelude::*, px, size,
};
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
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
    ]
);

pub const TAB_SIZE: usize = 4;
const DEFAULT_FONT_SIZE: f32 = 14.;
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
        KeyBinding::new("secondary-=", IncreaseFontSize, ctx),
        KeyBinding::new("secondary-+", IncreaseFontSize, ctx),
        KeyBinding::new("secondary--", DecreaseFontSize, ctx),
        KeyBinding::new("secondary-0", ResetFontSize, ctx),
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
    selecting: bool,
    pub font_size: Pixels,
}

impl Editor {
    pub fn new(buffer: Buffer, path: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let highlighter =
            path.as_deref().and_then(Path::extension).is_some_and(|ext| ext == "rs").then(Highlighter::rust);
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
            selecting: false,
            font_size: px(DEFAULT_FONT_SIZE),
        };
        editor.rehighlight();
        editor
    }

    /// Opens `path`, or an empty buffer that will be saved there if it doesn't exist yet.
    pub fn open(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        Self::new(Buffer::from_text(&text), Some(path), cx)
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

    pub fn language_name(&self) -> &'static str {
        if self.highlighter.is_some() { "Rust" } else { "Plain text" }
    }

    /// Zero-based line and column of the caret.
    pub fn caret_point(&self) -> (usize, usize) {
        self.buffer.point(self.selection.head)
    }

    pub fn line_height(&self) -> Pixels {
        (self.font_size * 1.7).round()
    }

    fn rehighlight(&mut self) {
        if let Some(highlighter) = &mut self.highlighter {
            self.spans = highlighter.highlight(&self.buffer.to_string());
        }
    }

    /// Marks the caret as just used: it stops blinking and the view follows it.
    fn touch(&mut self, cx: &mut Context<Self>) {
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
        let class = self.buffer.char_at(offset).map(char_class);
        let mut start = offset;
        let mut end = offset;
        while start > 0 && self.buffer.char_at(start - 1).map(char_class) == class {
            start -= 1;
        }
        while end < self.buffer.len_chars() && self.buffer.char_at(end).map(char_class) == class {
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
        self.record_undo(kind);
        let end = self.buffer.replace(range, text);
        self.selection = Selection::caret(end);
        self.marked = None;
        self.goal_column = None;
        self.rehighlight();
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    fn delete_or(&mut self, range: impl FnOnce(&Self) -> Range<usize>, cx: &mut Context<Self>) {
        let range = if self.selection.is_empty() { range(self) } else { self.selection.range() };
        if !range.is_empty() {
            self.edit(range, "", EditKind::Deleting, cx);
        }
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
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
        self.rehighlight();
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = &self.path else { return };
        match std::fs::write(path, self.buffer.to_string()) {
            Ok(()) => {
                self.buffer.mark_saved();
                cx.notify();
            }
            Err(err) => eprintln!("null: couldn't save {}: {err}", path.display()),
        }
    }

    fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.font_size = px(size.clamp(9., 32.));
        self.caret.placed = false;
        self.touch(cx);
    }

    fn increase_font_size(&mut self, _: &IncreaseFontSize, _: &mut Window, cx: &mut Context<Self>) {
        self.set_font_size(f32::from(self.font_size) + 1., cx);
    }

    fn decrease_font_size(&mut self, _: &DecreaseFontSize, _: &mut Window, cx: &mut Context<Self>) {
        self.set_font_size(f32::from(self.font_size) - 1., cx);
    }

    fn reset_font_size(&mut self, _: &ResetFontSize, _: &mut Window, cx: &mut Context<Self>) {
        self.set_font_size(DEFAULT_FONT_SIZE, cx);
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

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let offset = self.offset_at(event.position);
        match event.click_count {
            2 => {
                let word = self.word_at(offset);
                self.selection = Selection { anchor: word.start, head: word.end };
            }
            3 => {
                let (line, _) = self.buffer.point(offset);
                let end = if line + 1 < self.buffer.len_lines() {
                    self.buffer.line_to_char(line + 1)
                } else {
                    self.buffer.len_chars()
                };
                self.selection = Selection { anchor: self.buffer.line_to_char(line), head: end };
            }
            _ if event.modifiers.shift => self.selection.head = offset,
            _ => self.selection = Selection::caret(offset),
        }
        self.goal_column = None;
        self.selecting = true;
        self.touch(cx);
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            self.selection.head = self.offset_at(event.position);
            self.touch(cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
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
        self.edit(range, text, EditKind::Typing, cx);
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
        self.rehighlight();
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
        div()
            .key_context("Editor")
            .track_focus(&self.focus_handle)
            .size_full()
            .cursor(CursorStyle::IBeam)
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
            .on_action(cx.listener(Self::increase_font_size))
            .on_action(cx.listener(Self::decrease_font_size))
            .on_action(cx.listener(Self::reset_font_size))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(EditorElement::new(cx.entity()))
    }
}
