//! Changes the language server works out: renaming a symbol everywhere, formatting
//! the file, finding where something is used.

use super::{EditKind, Editor, EditorEvent, Selection};
use crate::buffer::Buffer;
use crate::fonts::CodeFont;
use crate::settings::Settings;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use gpui::{
    AnyElement, App, Context, Entity, Focusable, KeyBinding, Subscription, Window, actions, anchored, deferred, div,
    point, prelude::*, px,
};
use lsp_types::{Position, TextEdit};
use std::ops::Range;

actions!(
    refactor,
    [
        RenameSymbol,
        FindReferences,
        FormatDocument,
        FormatSelection,
        ConfirmRename,
        CancelRename,
        InsertTableOfContents,
        InsertFootnote
    ]
);

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
pub(super) fn edit_ranges<'a>(buffer: &Buffer, edits: &'a [TextEdit]) -> Vec<(Range<usize>, &'a str)> {
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
pub(super) fn caret_after(caret: usize, edits: &[(Range<usize>, &str)]) -> Option<usize> {
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
                        .code_font(cx)
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

    /// ⌥⇧F: the file, or with some text selected, just that.
    pub(super) fn format_document(&mut self, _: &FormatDocument, _: &mut Window, cx: &mut Context<Self>) {
        // Markdown without a server: its tables lined up. JSON: laid out by Null itself.
        if self.is_markdown() && !self.served(cx) {
            return self.align_markdown_tables(cx);
        }
        if self.language_name() == "JSON" && !self.served(cx) {
            return self.format_json(cx);
        }
        if let Some(message) = self.not_ready_message(cx) {
            return self.show_notice(self.selection.head, message, cx);
        }
        if !self.selection.is_empty() && self.extra.is_empty() {
            return self.format_selection_now(cx);
        }
        self.format_then(false, cx);
    }

    /// Markdown: the next footnote's mark at the caret (`[^3]`), its note started at the end of
    /// the document (with the notes there, if any), and the caret there to write it; ⌃- goes
    /// back. One undo step.
    pub(super) fn insert_footnote(&mut self, _: &InsertFootnote, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.selection.range().end;
        if !self.is_markdown() {
            return self.show_notice(at, "Footnotes are for Markdown files.".into(), cx);
        }
        let text = self.buffer.to_string();
        let n = crate::markdown_view::next_footnote(&text);
        let ending = self.style.line_ending.text();
        let content = text.trim_end();
        let content_end = content.chars().count();
        let last = content.lines().last().unwrap_or("");
        // After the notes already at the end, else after a blank line.
        let gap = if content.is_empty() {
            String::new()
        } else if last.starts_with("[^") {
            ending.to_string()
        } else {
            ending.repeat(2)
        };
        let label = format!("[^{n}]");
        let note = format!("{gap}{label}: ");
        let after = if text.ends_with(['\n', '\r']) { ending } else { "" };
        // The mark: at the caret (after the selection), or at the text's end when the caret
        // is past it.
        let mark_at = at.min(content_end);
        let from = self.caret_point();
        // At the very end, the mark and the note are one edit (two at one place would overwrite).
        let edits = if mark_at == content_end {
            vec![(content_end..self.buffer.len_chars(), format!("{label}{note}{after}"))]
        } else {
            vec![(mark_at..mark_at, label.clone()), (content_end..self.buffer.len_chars(), format!("{note}{after}"))]
        };
        self.apply_char_edits(edits, cx);
        let caret = content_end + label.chars().count() + note.chars().count();
        self.selection = super::Selection::caret(caret);
        cx.emit(EditorEvent::Jumped { from });
        self.touch(cx);
    }

    /// Markdown: a list of links to the headings, at the caret; or, where Null wrote one
    /// before, that one brought up to date.
    pub(super) fn insert_table_of_contents(
        &mut self,
        _: &InsertTableOfContents,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = self.selection.head;
        if !self.is_markdown() {
            return self.show_notice(at, "A table of contents is for Markdown files.".into(), cx);
        }
        let toc = crate::markdown_view::table_of_contents(&self.buffer.to_string());
        if toc.is_empty() {
            return self.show_notice(at, "No headings to list yet.".into(), cx);
        }
        let ending = self.style.line_ending.text();
        let text = toc.join(ending);
        // The one written before: its end mark, and the start mark nearest above it (another
        // tool's `<!-- toc -->` without Null's end isn't one), neither inside a code fence.
        let lines = self.buffer.len_lines();
        let mark = |i: usize, m: &str| self.buffer.line_text(i).trim() == m && !self.in_fence(i);
        let end = (0..lines).find(|&i| mark(i, crate::markdown_view::TOC_END));
        let start = end.and_then(|e| (0..e).rev().find(|&i| mark(i, crate::markdown_view::TOC_START)));
        if let (Some(start), Some(end)) = (start, end) {
            let range = self.buffer.line_to_char(start)..self.buffer.line_to_char(end) + self.buffer.line_len(end);
            self.edit(range, &text, EditKind::Other, cx);
            return self.show_notice(at, "Brought the table of contents up to date.".into(), cx);
        }
        let range = self.selection.range();
        let (_, column) = self.buffer.point(range.start);
        let text = if column > 0 { format!("{ending}{ending}{text}{ending}") } else { format!("{text}{ending}") };
        self.edit(range, &text, EditKind::Other, cx);
    }

    /// Every table of a Markdown file with its columns lined up, as one undo step.
    fn align_markdown_tables(&mut self, cx: &mut Context<Self>) {
        let tables = crate::markdown_view::aligned_tables(&self.buffer.to_string());
        if tables.is_empty() {
            let at = self.selection.head;
            return self.show_notice(at, "Nothing to tidy: the tables are lined up.".into(), cx);
        }
        let (line, column) = self.buffer.point(self.selection.head);
        self.record_undo(EditKind::Other);
        for (lines, new) in tables.into_iter().rev() {
            let start = self.buffer.line_to_char(lines.start);
            let last = lines.end - 1;
            let end = self.buffer.line_to_char(last) + self.buffer.line_len(last);
            self.buffer.replace(start..end, &new.join(self.style.line_ending.text()));
        }
        let line = line.min(self.buffer.len_lines().saturating_sub(1));
        self.selection = Selection::caret(self.buffer.offset(line, column.min(self.buffer.line_len(line))));
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    /// JSON laid out by Null (see `json_format`): one undo, the caret kept on its line. With
    /// text selected, just that, lined up with where it starts.
    fn format_json(&mut self, cx: &mut Context<Self>) {
        let unit = self.style.indent.unit();
        let newline = self.style.line_ending.text();
        let at = self.selection.head;
        if !self.selection.is_empty() && self.extra.is_empty() {
            let range = self.selection.range();
            let selected = self.buffer.slice(range.clone());
            let (line, _) = self.buffer.point(range.start);
            let indent: String = self.buffer.line_text(line).chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            let formatted = match crate::json_format::format(&selected, &unit, newline) {
                Ok(formatted) => formatted.replace(newline, &format!("{newline}{indent}")),
                Err(problem) => {
                    let message = format!("Can't format the selection: {}.", problem.message);
                    return self.show_notice(at, message, cx);
                }
            };
            // A selection ending with its line break keeps it, not an indent after it.
            let formatted = match formatted.strip_suffix(&indent) {
                Some(trimmed) if selected.ends_with('\n') && !indent.is_empty() => trimmed.to_string(),
                _ => formatted,
            };
            let len = formatted.chars().count();
            self.edit(range.clone(), &formatted, EditKind::Other, cx);
            self.selection = Selection { anchor: range.start, head: range.start + len };
            return self.touch(cx);
        }
        let text = self.buffer.to_string();
        let formatted = match crate::json_format::format(&text, &unit, newline) {
            Ok(formatted) => formatted,
            Err(problem) => {
                let message = format!("Can't format: {} (line {}).", problem.message, problem.line + 1);
                return self.show_notice(at, message, cx);
            }
        };
        if formatted == text {
            return self.show_notice(at, "Already formatted.".into(), cx);
        }
        // Only what changed is replaced, so marks and folds before and after stay put.
        let prefix = text.chars().zip(formatted.chars()).take_while(|(a, b)| a == b).count();
        let (old_len, new_len) = (text.chars().count(), formatted.chars().count());
        let suffix = text
            .chars()
            .rev()
            .zip(formatted.chars().rev())
            .take_while(|(a, b)| a == b)
            .count()
            .min(old_len - prefix)
            .min(new_len - prefix);
        let middle: String = formatted.chars().skip(prefix).take(new_len - prefix - suffix).collect();
        let (line, column) = self.buffer.point(at);
        self.edit(prefix..old_len - suffix, &middle, EditKind::Other, cx);
        let line = line.min(self.buffer.len_lines().saturating_sub(1));
        self.selection = Selection::caret(self.buffer.offset(line, column.min(self.buffer.line_len(line))));
        self.touch(cx);
    }

    pub(super) fn format_selection(&mut self, _: &FormatSelection, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(message) = self.not_ready_message(cx) {
            return self.show_notice(self.selection.head, message, cx);
        }
        if self.selection.is_empty() {
            return self.show_notice(self.selection.head, "Select the code to format.".into(), cx);
        }
        self.format_selection_now(cx);
    }

    /// Formats the selected code, if the language server can format part of a file.
    /// Pasted code formatted (Format on paste), quietly: by a server that formats ranges,
    /// before anything else is typed.
    pub(super) fn format_pasted(&mut self, pasted: std::ops::Range<usize>, cx: &mut Context<Self>) {
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else { return };
        if pasted.is_empty() || !lsp.read(cx).formats_ranges(&path) {
            return;
        }
        let range = lsp_types::Range { start: self.lsp_position(pasted.start), end: self.lsp_position(pasted.end) };
        // The revision, not the version: an undo takes the version back, so the same number
        // could stand for other text.
        let revision = self.buffer.revision();
        let request = lsp.read(cx).format_part(
            &path,
            Some(range),
            self.style.indent.width() as u32,
            self.style.indent != crate::file_style::Indent::Tabs,
        );
        self.paste_format_task = Some(cx.spawn(async move |this, cx| {
            let edits = request.await;
            this.update(cx, |this, cx| {
                // Typed on (or undone) since: the paste stays as it is.
                if this.buffer.revision() == revision && !edits.is_empty() {
                    this.apply_lsp_edits(&edits, cx);
                }
            })
            .ok();
        }));
    }

    fn format_selection_now(&mut self, cx: &mut Context<Self>) {
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else { return };
        let at = self.selection.head;
        if !lsp.read(cx).formats_ranges(&path) {
            let label = crate::lsp_store::LspStore::language_label(&path).unwrap_or("This language");
            let message =
                format!("{label} support formats whole files only: with nothing selected, it formats this one.");
            return self.show_notice(at, message, cx);
        }
        let selected = self.selection.range();
        let range = lsp_types::Range { start: self.lsp_position(selected.start), end: self.lsp_position(selected.end) };
        let revision = self.buffer.revision();
        let request = lsp.read(cx).format_part(
            &path,
            Some(range),
            self.style.indent.width() as u32,
            self.style.indent != crate::file_style::Indent::Tabs,
        );
        self.format_task = Some(cx.spawn(async move |this, cx| {
            let edits = request.await;
            this.update(cx, |this, cx| {
                if this.buffer.revision() != revision {
                    return;
                }
                if edits.is_empty() {
                    return this.show_notice(at, "Already formatted.".into(), cx);
                }
                this.apply_lsp_edits(&edits, cx);
            })
            .ok();
        }));
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
        // A paste still being formatted: the whole file's formatting covers it (and its
        // answer, landing first, would make this one look out of date).
        self.paste_format_task = None;
        let revision = self.buffer.revision();
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
                if let Some(edits) = edits.filter(|_| this.buffer.revision() == revision) {
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

    /// JSON with no server to format it: Null lays it out, in one undo; broken JSON is left
    /// as it is, with where it's broken.
    #[gpui::test]
    fn json_is_formatted_without_a_server(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let text = "{\"a\":[1,2],\n\"b\":{}}\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some("a.json".into()), cx));
        e.update_in(cx, |e, window, cx| window.focus(&gpui::Focusable::focus_handle(e, cx)));
        cx.simulate_keystrokes("alt-shift-f");
        e.read_with(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "{\n    \"a\": [\n        1,\n        2\n    ],\n    \"b\": {}\n}\n");
        });
        cx.simulate_keystrokes("cmd-z");
        e.read_with(cx, |e, _| assert_eq!(e.buffer.to_string(), text, "one undo"));
        e.update(cx, |e, cx| {
            let end = e.buffer.len_chars();
            e.edit(end..end, "]", EditKind::Other, cx);
        });
        cx.simulate_keystrokes("alt-shift-f");
        e.read_with(cx, |e, _| assert_eq!(e.buffer.to_string(), format!("{text}]"), "broken: left alone"));
        // A selection: just it, lined up under its line.
        e.update(cx, |e, cx| {
            let all = e.buffer.len_chars();
            e.edit(0..all, "{\n    \"a\": {\"b\":[1,2]},\n    \"c\": 3\n}\n", EditKind::Other, cx);
            let start = e.buffer.to_string().find("{\"b").unwrap();
            e.selection = Selection { anchor: start, head: start + "{\"b\":[1,2]}".len() };
        });
        cx.simulate_keystrokes("alt-shift-f");
        e.read_with(cx, |e, _| {
            assert_eq!(
                e.buffer.to_string(),
                "{\n    \"a\": {\n        \"b\": [\n            1,\n            2\n        ]\n    },\n    \"c\": 3\n}\n"
            )
        });
    }
}
