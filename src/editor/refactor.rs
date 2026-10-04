//! Changes the language server works out: renaming a symbol everywhere, formatting
//! the file, finding where something is used.

use super::{EditKind, Editor, EditorEvent, Selection};
use crate::buffer::Buffer;
use crate::settings::Settings;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use gpui::{
    AnyElement, App, Context, Entity, Focusable, KeyBinding, Subscription, Window, actions, anchored, deferred, div,
    point, prelude::*, px,
};
use lsp_types::{Position, TextEdit};
use std::ops::Range;

actions!(refactor, [RenameSymbol, FindReferences, FormatDocument, ConfirmRename, CancelRename]);

pub fn bind_keys(cx: &mut App) {
    let editor = Some("Editor");
    cx.bind_keys([
        KeyBinding::new("secondary-r", RenameSymbol, editor),
        KeyBinding::new("f2", RenameSymbol, editor),
        KeyBinding::new("shift-f12", FindReferences, editor),
        KeyBinding::new("alt-shift-f", FormatDocument, editor),
        KeyBinding::new("enter", ConfirmRename, Some("RenameField")),
        KeyBinding::new("escape", CancelRename, Some("RenameField")),
    ]);
}

/// Formatting before a save waits at most this long, then saves anyway.
const FORMAT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// The name being edited in place, over the word itself.
pub(super) struct Renaming {
    input: Entity<TextInput>,
    word: Range<usize>,
    _subscription: Subscription,
}

/// Applies a server's edits to a buffer, last first so earlier positions stay right.
/// Positions are lines and UTF-16 columns, as servers count.
pub fn apply_edits(buffer: &mut Buffer, edits: &[TextEdit]) {
    let mut ranges = edit_ranges(buffer, edits);
    ranges.sort_by_key(|(r, _)| std::cmp::Reverse((r.start, r.end)));
    for (range, text) in ranges {
        buffer.replace(range, text);
    }
}

/// The character ranges a server's edits replace, with their new text.
fn edit_ranges<'a>(buffer: &Buffer, edits: &'a [TextEdit]) -> Vec<(Range<usize>, &'a str)> {
    let offset = |p: Position| {
        let line = p.line as usize;
        if line >= buffer.len_lines() {
            return buffer.len_chars();
        }
        buffer.offset(line, buffer.utf16_to_column(line, p.character as usize))
    };
    edits.iter().map(|e| (offset(e.range.start)..offset(e.range.end), e.new_text.as_str())).collect()
}

/// Where `caret` ends up once `edits` are applied: shifted by the edits before it, or
/// None when one of them rewrites the text around it.
fn caret_after(caret: usize, edits: &[(Range<usize>, &str)]) -> Option<usize> {
    let mut shift = 0isize;
    for (range, text) in edits {
        if range.start < caret && caret < range.end {
            return None;
        }
        if range.end <= caret && range.start < caret {
            shift += text.chars().count() as isize - range.len() as isize;
        }
    }
    Some((caret as isize + shift).max(0) as usize)
}

impl Editor {
    /// Applies a server's edits as one undo step.
    pub fn apply_lsp_edits(&mut self, edits: &[TextEdit], cx: &mut Context<Self>) {
        if edits.is_empty() {
            return;
        }
        // The caret stays with its text (below an added import, say); where an edit
        // rewrites the text around it (formatting), it keeps its line and column.
        let (line, col) = self.caret_point();
        let moved = caret_after(self.selection.head, &edit_ranges(&self.buffer, edits));
        self.record_undo(EditKind::Other);
        apply_edits(&mut self.buffer, edits);
        self.single_cursor();
        let caret = moved.unwrap_or_else(|| self.buffer.offset(line, col)).min(self.buffer.len_chars());
        self.selection = Selection::caret(caret);
        self.goal_column = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    // ---------- rename ----------

    pub(super) fn rename_symbol(&mut self, _: &RenameSymbol, window: &mut Window, cx: &mut Context<Self>) {
        let word = self.word_at(self.selection.head);
        if word.is_empty() || !self.buffer.slice(word.clone()).chars().any(|c| c.is_alphanumeric() || c == '_') {
            return;
        }
        if let Some(message) = self.not_ready_message(cx) {
            return self.show_notice(word.start, message, cx);
        }
        let name = self.buffer.slice(word.clone());
        let input = cx.new(|cx| {
            let mut input = TextInput::new("New name", cx);
            input.set_text(&name, cx);
            input
        });
        let subscription = cx.subscribe(&input, |_, _, TextInputEvent::Changed, cx| cx.notify());
        window.focus(&input.focus_handle(cx));
        self.close_hover(cx);
        self.close_completion(cx);
        self.renaming = Some(Renaming { input, word, _subscription: subscription });
        cx.notify();
    }

    fn confirm_rename(&mut self, _: &ConfirmRename, window: &mut Window, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take() else { return };
        window.focus(&self.focus_handle);
        let new_name = renaming.input.read(cx).text().trim().to_string();
        if new_name.is_empty() || new_name == self.buffer.slice(renaming.word.clone()) {
            return cx.notify();
        }
        cx.emit(EditorEvent::Rename { position: self.lsp_position(renaming.word.start), new_name });
        cx.notify();
    }

    fn cancel_rename(&mut self, _: &CancelRename, window: &mut Window, cx: &mut Context<Self>) {
        self.renaming = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// The field sits right over the word, in the code's own font: editing the name in place.
    pub(super) fn render_rename(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let renaming = self.renaming.as_ref()?;
        let bounds = self.caret_bounds(renaming.word.start)?;
        let theme = cx.global::<Theme>();
        let code_font = cx.global::<crate::fonts::Fonts>().code.clone();
        let chars = renaming.input.read(cx).text().chars().count().max(8) as f32;
        let width = self.layout.as_ref().map_or(px(200.), |l| l.char_width * (chars + 4.));
        Some(
            deferred(
                anchored().position(point(bounds.left() - px(5.), bounds.top() - px(3.))).child(
                    div()
                        .key_context("RenameField")
                        .on_action(cx.listener(Self::confirm_rename))
                        .on_action(cx.listener(Self::cancel_rename))
                        .occlude()
                        .w(width)
                        .px(px(4.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(theme.caret)
                        .bg(theme.raised)
                        .shadow_md()
                        .font_family(code_font)
                        .text_size(self.font_size)
                        .line_height(self.line_height())
                        .child(renaming.input.clone()),
                ),
            )
            .into_any_element(),
        )
    }

    // ---------- references ----------

    pub(super) fn find_references(&mut self, _: &FindReferences, _: &mut Window, cx: &mut Context<Self>) {
        self.find_references_at(self.selection.head, cx);
    }

    pub(super) fn find_references_at(&mut self, offset: usize, cx: &mut Context<Self>) {
        if let Some(message) = self.not_ready_message(cx) {
            return self.show_notice(offset, message, cx);
        }
        let word = self.word_at(offset);
        cx.emit(EditorEvent::FindReferences { position: self.lsp_position(word.start), name: self.buffer.slice(word) });
    }

    // ---------- format ----------

    pub(super) fn format_document(&mut self, _: &FormatDocument, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(message) = self.not_ready_message(cx) {
            return self.show_notice(self.selection.head, message, cx);
        }
        self.format_then(false, cx);
    }

    /// Formats the file with its language server, then saves it if `save`. A server
    /// that takes too long (or can't format) doesn't hold the save back.
    fn format_then(&mut self, save: bool, cx: &mut Context<Self>) {
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else {
            if save {
                self.save_to_disk(cx);
            }
            return;
        };
        let version = self.buffer.version();
        let request = lsp.read(cx).format(
            &path,
            self.style.indent.width() as u32,
            self.style.indent != crate::file_style::Indent::Tabs,
        );
        self.format_task = Some(cx.spawn(async move |this, cx| {
            let timeout = cx.background_executor().timer(FORMAT_TIMEOUT);
            let edits = futures::select_biased! {
                edits = futures::FutureExt::fuse(request) => Some(edits),
                _ = futures::FutureExt::fuse(timeout) => None,
            };
            this.update(cx, |this, cx| {
                // Typing since the request started wins over the server's view of the text.
                if let Some(edits) = edits.filter(|_| this.buffer.version() == version) {
                    if !save && edits.is_empty() {
                        this.show_notice(this.selection.head, "Already formatted.".into(), cx);
                    }
                    this.apply_lsp_edits(&edits, cx);
                }
                if save {
                    this.save_to_disk(cx);
                }
            })
            .ok();
        }));
    }

    /// Saving from the keyboard: formats first when that's switched on.
    pub fn save_from_keyboard(&mut self, cx: &mut Context<Self>) {
        if cx.global::<Settings>().format_on_save && self.lsp.is_some() && self.path.is_some() {
            self.format_then(true, cx);
        } else {
            self.save_to_disk(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Range as LspRange;

    fn edit(sl: u32, sc: u32, el: u32, ec: u32, text: &str) -> TextEdit {
        TextEdit { range: LspRange { start: Position::new(sl, sc), end: Position::new(el, ec) }, new_text: text.into() }
    }

    #[test]
    fn applies_several_edits_in_any_order() {
        let mut buffer = Buffer::from_text("let count = 1;\nprint(count);\n");
        // A rename's edits, given first-to-last: each lands where it was meant to.
        apply_edits(&mut buffer, &[edit(0, 4, 0, 9, "total"), edit(1, 6, 1, 11, "total")]);
        assert_eq!(buffer.to_string(), "let total = 1;\nprint(total);\n");
        // A formatter's edit at the end of the file.
        apply_edits(&mut buffer, &[edit(2, 0, 2, 0, "\n")]);
        assert_eq!(buffer.to_string(), "let total = 1;\nprint(total);\n\n");
    }

    #[test]
    fn the_caret_follows_its_text() {
        let buffer = Buffer::from_text("fn main() {\n    x\n}\n");
        let caret = buffer.offset(1, 5);
        // An import added above: the caret moves down with its line.
        let import = [edit(0, 0, 0, 0, "use a::b;\n")];
        assert_eq!(caret_after(caret, &edit_ranges(&buffer, &import)), Some(caret + 10));
        // An edit after it leaves it alone; one around it gives up.
        assert_eq!(caret_after(caret, &edit_ranges(&buffer, &[edit(2, 0, 2, 1, "}}")])), Some(caret));
        assert_eq!(caret_after(caret, &edit_ranges(&buffer, &[edit(0, 0, 3, 0, "")])), None);
    }
}
