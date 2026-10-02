//! Cmd/Ctrl+I in the editor: pick what to change, show the card, apply the result.

use super::{EditKind, Editor, EditorEvent, Selection};
use crate::inline_assist::{AssistTarget, InlineAssist, InlineAssistEvent};
use gpui::{AnyElement, Context, Corner, Focusable, Window, anchored, deferred, point, prelude::*, px};
use std::ops::Range;

/// Items a Cmd+I without a selection works on, smallest first.
const RUST_SCOPES: &[&str] =
    &["function_item", "struct_item", "enum_item", "impl_item", "trait_item", "macro_definition", "mod_item"];

impl Editor {
    /// The lines Cmd+I changes: the selected lines, else the Rust item around the
    /// caret (with its doc comments and attributes), else the paragraph around it.
    fn assist_lines(&self) -> Range<usize> {
        let selection = self.selection.range();
        if !selection.is_empty() {
            let (start, _) = self.buffer.point(selection.start);
            let (end_line, end_col) = self.buffer.point(selection.end);
            let end = if end_col == 0 && end_line > start { end_line } else { end_line + 1 };
            return start..end;
        }
        let (caret_line, _) = self.caret_point();
        if self.language_name() == "Rust"
            && let Some(lines) = self.rust_item_lines(caret_line)
        {
            return lines;
        }
        let blank = |l: usize| self.buffer.line_text(l).trim().is_empty();
        let mut start = caret_line;
        while start > 0 && !blank(start - 1) {
            start -= 1;
        }
        let mut end = caret_line + 1;
        while end < self.buffer.len_lines() && !blank(end) {
            end += 1;
        }
        start..end
    }

    fn rust_item_lines(&self, line: usize) -> Option<Range<usize>> {
        let text = self.buffer.to_string();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).ok()?;
        let tree = parser.parse(&text, None)?;
        let byte = self.buffer.line_to_byte(line) + self.buffer.line_text(line).len()
            - self.buffer.line_text(line).trim_start().len();
        let mut node = tree.root_node().descendant_for_byte_range(byte, byte)?;
        loop {
            if RUST_SCOPES.contains(&node.kind()) {
                break;
            }
            node = node.parent()?;
        }
        let mut start = node.start_position().row;
        let end = node.end_position().row + 1;
        // Bring the doc comments and attributes just above along.
        while start > 0 {
            let above = self.buffer.line_text(start - 1);
            let above = above.trim_start();
            if above.starts_with("///") || above.starts_with("#[") || above.starts_with("//!") {
                start -= 1;
            } else {
                break;
            }
        }
        Some(start..end.min(self.buffer.len_lines()))
    }

    fn line_range_chars(&self, lines: &Range<usize>) -> Range<usize> {
        let start = self.buffer.line_to_char(lines.start);
        let end = if lines.end >= self.buffer.len_lines() {
            self.buffer.len_chars()
        } else {
            self.buffer.line_to_char(lines.end)
        };
        start..end
    }

    pub(super) fn open_inline_assist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // AI switched off: no AI anywhere, not even a card saying so.
        if !cx.global::<crate::settings::Settings>().ai.enabled {
            return;
        }
        self.close_completion(cx);
        self.close_hover(cx);
        let lines = self.assist_lines();
        let original = self.buffer.slice(self.line_range_chars(&lines));
        let target = AssistTarget {
            path: self.path.clone(),
            language: self.language_name(),
            context: InlineAssist::trim_context(&self.buffer.to_string(), &lines),
            lines,
            original,
        };
        let card = cx.new(|cx| InlineAssist::new(target, cx));
        let subscription = cx.subscribe_in(&card, window, |this, card, event, window, cx| {
            let lines = card.read(cx).lines();
            match event {
                InlineAssistEvent::Accept(text) => this.apply_assist(lines, text.clone(), cx),
                InlineAssistEvent::Dismiss => {}
            }
            this.assist = None;
            window.focus(&this.focus_handle);
            cx.notify();
        });
        window.focus(&card.focus_handle(cx));
        self.assist = Some((card, subscription));
        cx.notify();
    }

    fn apply_assist(&mut self, lines: Range<usize>, text: String, cx: &mut Context<Self>) {
        let range = self.line_range_chars(&lines);
        self.record_undo(EditKind::Other);
        let end = self.buffer.replace(range.clone(), &text);
        // Select what changed, so it's easy to see (and to adjust).
        self.single_cursor();
        self.selection = Selection { anchor: range.start, head: end };
        self.goal_column = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    /// Lines highlighted while the card is open.
    pub fn assist_target(&self, cx: &gpui::App) -> Option<Range<usize>> {
        self.assist.as_ref().map(|(card, _)| card.read(cx).lines())
    }

    pub(super) fn render_assist(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (card, _) = self.assist.as_ref()?;
        let lines = card.read(cx).lines();
        let layout = self.layout.as_ref()?;
        // Sit just above the code it changes; slide below when there's no room above.
        let top = layout.text_origin.y + layout.line_height * self.wrap.first_row(lines.start) as f32;
        let x = layout.text_bounds.left() + px(8.);
        let (corner, y) = if top - layout.text_bounds.top() > px(160.) {
            (Corner::BottomLeft, top - px(6.))
        } else {
            (Corner::TopLeft, top + layout.line_height * lines.len().min(3) as f32 + px(6.))
        };
        Some(
            deferred(
                anchored().anchor(corner).position(point(x, y)).snap_to_window_with_margin(px(8.)).child(card.clone()),
            )
            .into_any_element(),
        )
    }
}
