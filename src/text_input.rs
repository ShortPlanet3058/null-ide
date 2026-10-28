use crate::theme::Theme;
use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font, GlobalElementId, Hsla,
    InspectorElementId, KeyBinding, KeyContext, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, ShapedLine, SharedString, Style, TextAlign, TextRun, UTF16Selection, Window, WrappedLine,
    actions, div, fill, point, prelude::*, px, relative, size,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_input,
    [
        Backspace,
        BackspaceWord,
        BackspaceAll,
        Delete,
        DeleteToEnd,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Newline,
        Up,
        Down,
        SelectUp,
        SelectDown,
        TextStart,
        TextEnd
    ]
);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("TextInput");
    let mut keys = vec![
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("secondary-a", SelectAll, ctx),
        KeyBinding::new("secondary-v", Paste, ctx),
        KeyBinding::new("secondary-c", Copy, ctx),
        KeyBinding::new("secondary-x", Cut, ctx),
    ];
    if cfg!(target_os = "macos") {
        keys.extend([
            KeyBinding::new("alt-backspace", BackspaceWord, ctx),
            KeyBinding::new("cmd-backspace", BackspaceAll, ctx),
            KeyBinding::new("cmd-left", Home, ctx),
            KeyBinding::new("cmd-right", End, ctx),
            // The ⌃ keys of every macOS text field.
            KeyBinding::new("ctrl-a", Home, ctx),
            KeyBinding::new("ctrl-e", End, ctx),
            KeyBinding::new("ctrl-f", Right, ctx),
            KeyBinding::new("ctrl-b", Left, ctx),
            KeyBinding::new("ctrl-d", Delete, ctx),
            KeyBinding::new("ctrl-h", Backspace, ctx),
            KeyBinding::new("ctrl-k", DeleteToEnd, ctx),
        ]);
    } else {
        keys.push(KeyBinding::new("ctrl-backspace", BackspaceWord, ctx));
    }
    // A field of several lines: ⇧↵ starts one (↵ is the field's owner's), ↑↓ go between them.
    let several = Some("TextInput && multiline");
    keys.extend([
        KeyBinding::new("shift-enter", Newline, several),
        KeyBinding::new("up", Up, several),
        KeyBinding::new("down", Down, several),
        KeyBinding::new("shift-up", SelectUp, several),
        KeyBinding::new("shift-down", SelectDown, several),
    ]);
    if cfg!(target_os = "macos") {
        keys.extend([KeyBinding::new("cmd-up", TextStart, several), KeyBinding::new("cmd-down", TextEnd, several)]);
    } else {
        keys.extend([KeyBinding::new("ctrl-home", TextStart, several), KeyBinding::new("ctrl-end", TextEnd, several)]);
    }
    cx.bind_keys(keys);
}

pub enum TextInputEvent {
    Changed,
}

/// Room between the caret and the hint shown in an empty field.
const PLACEHOLDER_GAP: gpui::Pixels = px(4.);

/// The rows a field of several lines grows to; past them, it scrolls.
const MAX_ROWS: usize = 8;

/// A text field: one line (the command palette's), or several, wrapped to its width (see
/// [`TextInput::multiline`]).
pub struct TextInput {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    /// Byte offsets into `content`.
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    selecting: bool,
    /// Shows dots instead of the text, for secrets like API keys.
    pub masked: bool,
    /// Several lines (⇧↵ starts one, pasted lines stay), wrapped to the width.
    multiline: bool,
    /// Its text as last laid out, when it has several lines.
    laid: Option<Laid>,
    /// Rows scrolled past at the top (more than `MAX_ROWS`, the caret's kept in sight).
    scrolled: usize,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder: placeholder.into(),
            selected: 0..0,
            reversed: false,
            marked: None,
            last_layout: None,
            last_bounds: None,
            selecting: false,
            masked: false,
            multiline: false,
            laid: None,
            scrolled: 0,
        }
    }

    /// A field of several lines, wrapped to its width: it grows with them, up to `MAX_ROWS`.
    pub fn multiline(mut self) -> Self {
        self.multiline = true;
        self
    }

    /// The rows beyond its first it took when last drawn, in rows of `row_height` (a field
    /// of several lines grows with them; it's told, as a change, when that changes).
    pub fn rows_added(&self, row_height: Pixels) -> usize {
        let rows = self.rows_shown();
        match self.last_bounds {
            Some(bounds) if rows > 1 => {
                let added = bounds.size.height * ((rows - 1) as f32 / rows as f32);
                (added / row_height.max(px(1.))).ceil() as usize
            }
            _ => 0,
        }
    }

    fn rows_shown(&self) -> usize {
        self.laid.as_ref().map_or(1, |l| l.rows().min(MAX_ROWS))
    }

    /// Text as this field takes it: on one line, unless it has several.
    fn taken(&self, text: &str) -> String {
        if self.multiline { text.replace("\r\n", "\n").replace('\r', "\n") } else { text.replace(['\n', '\r'], " ") }
    }

    /// Where the line `offset` is on starts and ends (the whole text, on one line).
    fn line_start(&self, offset: usize) -> usize {
        self.content[..offset].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self, offset: usize) -> usize {
        self.content[offset..].find('\n').map_or(self.content.len(), |i| offset + i)
    }

    /// The place `rows` rows above (negative) or below `offset`, as it's laid out; past the
    /// first row, the text's start, past the last, its end.
    fn row_away(&self, offset: usize, rows: isize) -> usize {
        let Some(laid) = &self.laid else { return offset };
        let at = laid.point_for(offset);
        let y = at.y + laid.line_height * (rows as f32 + 0.5);
        if y < px(0.) {
            0
        } else if y >= laid.line_height * laid.rows() as f32 {
            self.content.len()
        } else {
            laid.index_at(point(at.x, y))
        }
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    /// Replaces the text and selects all of it, so typing starts over.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = self.taken(text);
        self.selected = 0..self.content.len();
        self.reversed = false;
        self.marked = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    /// Selects a byte range of the text, clamped to it.
    pub fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let end = range.end.min(self.content.len());
        self.selected = range.start.min(end)..end;
        self.reversed = false;
        cx.notify();
    }

    /// Like `set_text`, with the caret after it rather than all of it selected.
    pub fn set_text_caret_at_end(&mut self, text: &str, cx: &mut Context<Self>) {
        self.set_text(text, cx);
        self.selected = self.content.len()..self.content.len();
    }

    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.selected = 0..self.content.len();
        self.reversed = false;
        cx.notify();
    }

    fn cursor(&self) -> usize {
        if self.reversed { self.selected.start } else { self.selected.end }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected = offset..offset;
        self.reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).rev().find_map(|(i, _)| (i < offset).then_some(i)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).find_map(|(i, _)| (i > offset).then_some(i)).unwrap_or(self.content.len())
    }

    fn previous_word(&self, offset: usize) -> usize {
        self.content
            .split_word_bound_indices()
            .rev()
            .find_map(|(i, w)| (i < offset && !w.trim().is_empty()).then_some(i))
            .unwrap_or(0)
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        self.content.replace_range(range.clone(), text);
        let end = range.start + text.len();
        self.selected = end..end;
        self.reversed = false;
        self.marked = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.previous_boundary(self.cursor()), cx);
        }
        self.replace(self.selected.clone(), "", cx);
    }

    fn backspace_word(&mut self, _: &BackspaceWord, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.previous_word(self.cursor()), cx);
        }
        self.replace(self.selected.clone(), "", cx);
    }

    fn backspace_all(&mut self, _: &BackspaceAll, _: &mut Window, cx: &mut Context<Self>) {
        // (Several lines: to the start of the line.)
        let from = if self.multiline { self.line_start(self.selected.start) } else { 0 };
        let from = if from == self.selected.end { self.previous_boundary(from) } else { from };
        self.replace(from..self.selected.end, "", cx);
    }

    fn delete_to_end(&mut self, _: &DeleteToEnd, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.multiline { self.line_end(self.selected.start) } else { self.content.len() };
        let to = if to == self.selected.start { self.next_boundary(to) } else { to };
        self.replace(self.selected.start..to, "", cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.next_boundary(self.cursor()), cx);
        }
        self.replace(self.selected.clone(), "", cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selected.is_empty() { self.previous_boundary(self.cursor()) } else { self.selected.start };
        self.move_to(to, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selected.is_empty() { self.next_boundary(self.selected.end) } else { self.selected.end };
        self.move_to(to, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = 0..self.content.len();
        self.reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.multiline { self.line_start(self.cursor()) } else { 0 };
        self.move_to(to, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.multiline { self.line_end(self.cursor()) } else { self.content.len() };
        self.move_to(to, cx);
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.replace(self.selected.clone(), "\n", cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let from = if self.selected.is_empty() { self.cursor() } else { self.selected.start };
        self.move_to(self.row_away(from, -1), cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let from = if self.selected.is_empty() { self.cursor() } else { self.selected.end };
        self.move_to(self.row_away(from, 1), cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_away(self.cursor(), -1), cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_away(self.cursor(), 1), cx);
    }

    fn text_start(&mut self, _: &TextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn text_end(&mut self, _: &TextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = crate::system_clipboard::read(cx).and_then(|item| item.text()) {
            self.replace(self.selected.clone(), &self.taken(&text), cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            crate::system_clipboard::write(
                cx,
                ClipboardItem::new_string(self.content[self.selected.clone()].to_string()),
            );
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            crate::system_clipboard::write(
                cx,
                ClipboardItem::new_string(self.content[self.selected.clone()].to_string()),
            );
            self.replace(self.selected.clone(), "", cx);
        }
    }

    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        if let (Some(bounds), Some(laid)) = (&self.last_bounds, &self.laid) {
            let scrolled = point(px(0.), laid.line_height * self.scrolled as f32);
            return laid.index_at(position - bounds.origin + scrolled).min(self.content.len());
        }
        match (&self.last_bounds, &self.last_layout) {
            (Some(bounds), Some(line)) if !self.content.is_empty() => {
                crate::ui::index_at_x(line, position.x - bounds.left()).min(self.content.len())
            }
            _ => 0,
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        self.selecting = true;
        let index = self.index_for_position(event.position);
        if event.modifiers.shift { self.select_to(index, cx) } else { self.move_to(index, cx) }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.select_to(self.index_for_position(event.position), cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset.min(self.content.len())].encode_utf16().count()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut units = 0;
        for (i, c) in self.content.char_indices() {
            if units >= offset {
                return i;
            }
            units += c.len_utf16();
        }
        self.content.len()
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected), reversed: self.reversed })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range =
            range.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked.clone()).unwrap_or(self.selected.clone());
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        new_selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range =
            range.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked.clone()).unwrap_or(self.selected.clone());
        self.content.replace_range(range.clone(), text);
        self.marked = (!text.is_empty()).then_some(range.start..range.start + text.len());
        // The new selection is given in UTF-16 units relative to the inserted text.
        let byte_in_text = |units: usize| {
            let mut n = 0;
            text.char_indices()
                .find_map(|(i, c)| {
                    let at = (n >= units).then_some(i);
                    n += c.len_utf16();
                    at
                })
                .unwrap_or(text.len())
        };
        self.selected = new_selected
            .map(|r| range.start + byte_in_text(r.start)..range.start + byte_in_text(r.end))
            .unwrap_or(range.start + text.len()..range.start + text.len());
        self.reversed = false;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        if let Some(laid) = &self.laid {
            let at = laid.point_for(range.start) - point(px(0.), laid.line_height * self.scrolled as f32);
            let top_left = bounds.origin + at;
            return Some(Bounds::new(top_left, size(px(2.), laid.line_height)));
        }
        let layout = self.last_layout.as_ref()?;
        Some(Bounds::from_corners(
            point(bounds.left() + layout.x_for_index(range.start), bounds.top()),
            point(bounds.left() + layout.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.index_for_position(point)))
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut context = KeyContext::new_with_defaults();
        context.add("TextInput");
        if self.multiline {
            context.add("multiline");
        }
        div()
            .key_context(context)
            .track_focus(&self.focus_handle)
            .w_full()
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::backspace_word))
            .on_action(cx.listener(Self::backspace_all))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_to_end))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .when(self.multiline, |d| {
                d.on_action(cx.listener(Self::newline))
                    .on_action(cx.listener(Self::up))
                    .on_action(cx.listener(Self::down))
                    .on_action(cx.listener(Self::select_up))
                    .on_action(cx.listener(Self::select_down))
                    .on_action(cx.listener(Self::text_start))
                    .on_action(cx.listener(Self::text_end))
            })
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .child(TextInputElement { input: cx.entity() })
    }
}

struct TextInputElement {
    input: Entity<TextInput>,
}

struct InputPrepaint {
    line: ShapedLine,
    caret: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    /// A field of several lines: its text laid out, and the selection's piece on each row.
    laid: Option<(Laid, Vec<PaintQuad>)>,
}

/// One line of a field of several, wrapped to its width.
#[derive(Clone)]
struct LaidLine {
    /// Where it starts in the text (bytes).
    start: usize,
    line: WrappedLine,
    /// Where each of its rows starts, in it.
    rows: Vec<usize>,
}

impl LaidLine {
    /// Where its row `row` ends, in it.
    fn row_end(&self, row: usize) -> usize {
        self.rows.get(row + 1).copied().unwrap_or(self.line.len())
    }

    /// Its row `offset` (in it) is on: a row's start is on that row, not the end of the last.
    fn row_of(&self, offset: usize) -> usize {
        self.rows.iter().rposition(|&start| start <= offset).unwrap_or(0)
    }

    /// The x of `offset` (in it) on its row `row`.
    fn x(&self, row: usize, offset: usize) -> Pixels {
        let layout = &self.line.unwrapped_layout;
        layout.x_for_index(offset) - layout.x_for_index(self.rows[row])
    }
}

/// A field's text on several rows: its lines, each wrapped to the width.
#[derive(Clone)]
struct Laid {
    lines: Vec<LaidLine>,
    line_height: Pixels,
}

impl Laid {
    fn new(
        text: &str,
        font: Font,
        font_size: Pixels,
        color: Hsla,
        width: Option<Pixels>,
        line_height: Pixels,
        window: &Window,
    ) -> Self {
        let run =
            TextRun { len: text.len(), font, color, background_color: None, underline: None, strikethrough: None };
        let shaped = window
            .text_system()
            .shape_text(SharedString::from(text.to_string()), font_size, &[run], width, None)
            .unwrap_or_default();
        let mut start = 0;
        let lines = shaped
            .into_iter()
            .map(|line| {
                let layout = &line.unwrapped_layout;
                let rows = std::iter::once(0)
                    .chain(line.wrap_boundaries.iter().map(|b| layout.runs[b.run_ix].glyphs[b.glyph_ix].index))
                    .collect();
                let laid = LaidLine { start, rows, line };
                // (And its \n.)
                start += laid.line.len() + 1;
                laid
            })
            .collect();
        Self { lines, line_height }
    }

    fn rows(&self) -> usize {
        self.lines.iter().map(|l| l.rows.len()).sum::<usize>().max(1)
    }

    /// The line `offset` is on, and the rows above that line.
    fn line_of(&self, offset: usize) -> Option<(&LaidLine, usize)> {
        let mut above = 0;
        for (i, line) in self.lines.iter().enumerate() {
            if offset <= line.start + line.line.len() || i + 1 == self.lines.len() {
                return Some((line, above));
            }
            above += line.rows.len();
        }
        None
    }

    /// Where `offset` is drawn, from the top left of the text.
    fn point_for(&self, offset: usize) -> Point<Pixels> {
        let Some((line, above)) = self.line_of(offset) else { return point(px(0.), px(0.)) };
        let local = offset.saturating_sub(line.start).min(line.line.len());
        let row = line.row_of(local);
        point(line.x(row, local), self.line_height * (above + row) as f32)
    }

    /// The place nearest `position` (from the top left of the text).
    fn index_at(&self, position: Point<Pixels>) -> usize {
        let wanted = (position.y / self.line_height).floor().max(0.) as usize;
        let mut above = 0;
        for (i, line) in self.lines.iter().enumerate() {
            let rows = line.rows.len();
            if wanted < above + rows || i + 1 == self.lines.len() {
                let row = (wanted - above.min(wanted)).min(rows - 1);
                let layout = &line.line.unwrapped_layout;
                let x = position.x + layout.x_for_index(line.rows[row]);
                let end = line.row_end(row);
                let index = crate::ui::index_at_x(layout, x).clamp(line.rows[row], end);
                // On a wrapped row, its last place is before the space it was wrapped at.
                let index = if index == end && row + 1 < rows {
                    line.line.text[..end].char_indices().next_back().map_or(end, |(i, _)| i)
                } else {
                    index
                };
                return line.start + index;
            }
            above += rows;
        }
        0
    }

    /// The selection `range` as a quad on each row it's on (a line's end shown as a sliver).
    fn selection(&self, range: &Range<usize>, origin: Point<Pixels>, color: Hsla) -> Vec<PaintQuad> {
        let mut quads = Vec::new();
        let mut above = 0;
        for line in &self.lines {
            for row in 0..line.rows.len() {
                let (start, end) = (line.start + line.rows[row], line.start + line.row_end(row));
                let last_row = row + 1 == line.rows.len();
                if range.start <= end && range.end > start {
                    let from = range.start.max(start) - line.start;
                    let to = range.end.min(end) - line.start;
                    let mut right = line.x(row, to);
                    // Past the line's end: its \n is selected too.
                    if last_row && range.end > end {
                        right += px(6.);
                    }
                    let left = line.x(row, from);
                    if right > left {
                        let top = origin.y + self.line_height * (above + row) as f32;
                        quads.push(fill(
                            Bounds::from_corners(
                                point(origin.x + left, top),
                                point(origin.x + right, top + self.line_height),
                            ),
                            color,
                        ));
                    }
                }
            }
            above += line.rows.len();
        }
        quads
    }
}

impl IntoElement for TextInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl TextInputElement {
    /// Its rows having changed from `before`, it's told (once drawn: what holds it makes room).
    fn tell_rows(input: &TextInput, before: usize, cx: &mut Context<TextInput>) {
        if input.rows_shown() != before {
            let input = cx.entity();
            cx.defer(move |cx| input.update(cx, |_, cx| cx.notify()));
        }
    }
}

impl Element for TextInputElement {
    type RequestLayoutState = ();
    type PrepaintState = InputPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let input = self.input.read(cx);
        if !input.multiline || input.content.is_empty() {
            style.size.height = window.line_height().into();
            return (window.request_layout(style, [], cx), ());
        }
        // As many rows as the text needs at the width it gets, up to `MAX_ROWS`.
        let text = input.content.clone();
        let text_style = window.text_style();
        let (font, font_size) = (text_style.font(), text_style.font_size.to_pixels(window.rem_size()));
        let (color, line_height) = (text_style.color, window.line_height());
        let measure = move |known: gpui::Size<Option<Pixels>>,
                            available: gpui::Size<AvailableSpace>,
                            window: &mut Window,
                            _: &mut App| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let laid = Laid::new(&text, font.clone(), font_size, color, width, line_height, window);
            size(width.unwrap_or_default(), line_height * laid.rows().min(MAX_ROWS) as f32)
        };
        (window.request_measured_layout(style, measure), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let theme = cx.global::<Theme>();
        let input = self.input.read(cx);
        let style = window.text_style();
        let (text, color) = if input.content.is_empty() {
            (input.placeholder.clone(), theme.faint)
        } else if input.masked {
            (SharedString::from("•".repeat(input.content.chars().count())), style.color)
        } else {
            (SharedString::from(input.content.clone()), style.color)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        if input.multiline && !input.content.is_empty() {
            let line_height = window.line_height();
            let laid = Laid::new(
                &input.content,
                style.font(),
                font_size,
                style.color,
                Some(bounds.size.width),
                line_height,
                window,
            );
            // The caret's row kept in sight.
            let caret_row = (laid.point_for(input.cursor()).y / line_height).round() as usize;
            let shown = MAX_ROWS.min(laid.rows());
            let scrolled =
                input.scrolled.min(laid.rows() - shown).clamp(caret_row.saturating_sub(shown - 1), caret_row);
            let origin = bounds.origin - point(px(0.), line_height * scrolled as f32);
            let selection = laid.selection(&input.selected, origin, theme.selection);
            let caret = input.selected.is_empty().then(|| {
                let at = origin + laid.point_for(input.cursor());
                let caret_height = line_height * 0.8;
                fill(
                    Bounds::new(point(at.x, at.y + (line_height - caret_height) / 2.), size(px(2.), caret_height)),
                    theme.caret,
                )
            });
            self.input.update(cx, |input, _| input.scrolled = scrolled);
            return InputPrepaint {
                line: ShapedLine::default(),
                caret,
                selection: None,
                laid: Some((laid, selection)),
            };
        }
        let line = window.text_system().shape_line(text, font_size, &[run], None);

        let x = |offset: usize| {
            if input.content.is_empty() {
                px(0.)
            } else if input.masked {
                // Each character shows as one dot, which is 3 bytes long.
                line.x_for_index(input.content[..offset].chars().count() * '•'.len_utf8())
            } else {
                line.x_for_index(offset)
            }
        };
        let (selection, caret) = if input.selected.is_empty() {
            let caret_height = bounds.size.height * 0.8;
            let caret = fill(
                Bounds::new(
                    point(bounds.left() + x(input.cursor()), bounds.top() + (bounds.size.height - caret_height) / 2.),
                    size(px(2.), caret_height),
                ),
                theme.caret,
            );
            (None, Some(caret))
        } else {
            let selection = fill(
                Bounds::from_corners(
                    point(bounds.left() + x(input.selected.start), bounds.top()),
                    point(bounds.left() + x(input.selected.end), bounds.bottom()),
                ),
                theme.selection,
            );
            (Some(selection), None)
        };
        InputPrepaint { line, caret, selection, laid: None }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.input.clone()), cx);
        if let Some((laid, selection)) = prepaint.laid.take() {
            let scrolled = self.input.read(cx).scrolled;
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                for quad in selection {
                    window.paint_quad(quad);
                }
                let mut top = bounds.top() - laid.line_height * scrolled as f32;
                for line in &laid.lines {
                    line.line
                        .paint(point(bounds.left(), top), laid.line_height, TextAlign::Left, None, window, cx)
                        .ok();
                    top += laid.line_height * line.rows.len() as f32;
                }
                if focus_handle.is_focused(window)
                    && let Some(caret) = prepaint.caret.take()
                {
                    window.paint_quad(caret);
                }
            });
            self.input.update(cx, |input, cx| {
                let rows = input.rows_shown();
                input.laid = Some(laid);
                input.last_layout = None;
                input.last_bounds = Some(bounds);
                Self::tell_rows(input, rows, cx);
            });
            return;
        }
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }
        // The hint stands clear of the caret before it, not under it.
        let hint = self.input.read(cx).content.is_empty();
        let origin = if hint { bounds.origin + point(PLACEHOLDER_GAP, px(0.)) } else { bounds.origin };
        prepaint.line.paint(origin, window.line_height(), window, cx).ok();
        if focus_handle.is_focused(window)
            && let Some(caret) = prepaint.caret.take()
        {
            window.paint_quad(caret);
        }
        let line = prepaint.line.clone();
        let is_placeholder = self.input.read(cx).content.is_empty() || self.input.read(cx).masked;
        self.input.update(cx, |input, cx| {
            let rows = input.rows_shown();
            input.last_layout = (!is_placeholder).then_some(line);
            input.laid = None;
            input.last_bounds = Some(bounds);
            Self::tell_rows(input, rows, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn a_field_of_several_lines_keeps_them_and_moves_between_them(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Theme::oled());
            bind_keys(cx);
        });
        let (input, cx) = cx.add_window_view(|_, cx| TextInput::new("", cx).multiline());
        input.update_in(cx, |input, window, _| window.focus(&input.focus_handle));
        cx.simulate_input("first");
        cx.simulate_keystrokes("shift-enter");
        cx.simulate_input("second");
        cx.run_until_parked();
        assert_eq!(input.read_with(cx, |i, _| i.text().to_string()), "first\nsecond");
        // ↑ goes to the line above, at the same place; Home and End are the line's.
        cx.simulate_keystrokes("up");
        cx.simulate_input("!");
        cx.simulate_keystrokes("end");
        cx.simulate_input("?");
        assert_eq!(input.read_with(cx, |i, _| i.text().to_string()), "first!?\nsecond");
        cx.run_until_parked();
        cx.simulate_keystrokes("down home");
        cx.simulate_input(">");
        assert_eq!(input.read_with(cx, |i, _| i.text().to_string()), "first!?\n>second");
        // Text put in keeps its lines; a one-line field's goes on one.
        input.update(cx, |i, cx| i.set_text("a\r\nb", cx));
        assert_eq!(input.read_with(cx, |i, _| i.text().to_string()), "a\nb");
        let (single, cx) = cx.add_window_view(|_, cx| TextInput::new("", cx));
        single.update(cx, |i, cx| i.set_text("a\nb", cx));
        assert_eq!(single.read_with(cx, |i, _| i.text().to_string()), "a b");
    }
}
