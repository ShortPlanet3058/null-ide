use crate::theme::Theme;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, InspectorElementId, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine,
    SharedString, Style, TextRun, UTF16Selection, Window, actions, div, fill, point, prelude::*, px, relative, size,
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
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy
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
        ]);
    } else {
        keys.push(KeyBinding::new("ctrl-backspace", BackspaceWord, ctx));
    }
    cx.bind_keys(keys);
}

pub enum TextInputEvent {
    Changed,
}

/// A single-line text field, used by the command palette.
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
        self.content = text.replace(['\n', '\r'], " ");
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
        self.replace(0..self.selected.end, "", cx);
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
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace(self.selected.clone(), &text.replace(['\n', '\r'], " "), cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected.clone()].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected.clone()].to_string()));
            self.replace(self.selected.clone(), "", cx);
        }
    }

    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        match (&self.last_bounds, &self.last_layout) {
            (Some(bounds), Some(line)) if !self.content.is_empty() => {
                line.closest_index_for_x(position.x - bounds.left()).min(self.content.len())
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
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
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
        div()
            .key_context("TextInput")
            .track_focus(&self.focus_handle)
            .w_full()
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::backspace_word))
            .on_action(cx.listener(Self::backspace_all))
            .on_action(cx.listener(Self::delete))
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
}

impl IntoElement for TextInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
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
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
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
        InputPrepaint { line, caret, selection }
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
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }
        prepaint.line.paint(bounds.origin, window.line_height(), window, cx).ok();
        if focus_handle.is_focused(window)
            && let Some(caret) = prepaint.caret.take()
        {
            window.paint_quad(caret);
        }
        let line = prepaint.line.clone();
        let is_placeholder = self.input.read(cx).content.is_empty() || self.input.read(cx).masked;
        self.input.update(cx, |input, _| {
            input.last_layout = (!is_placeholder).then_some(line);
            input.last_bounds = Some(bounds);
        });
    }
}
