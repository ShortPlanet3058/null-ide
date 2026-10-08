//! Line-level editing commands: comments, indentation, moving and duplicating lines,
//! and the brackets and quotes that close themselves.

use super::{EditKind, Editor, EditorEvent, Selection};
use gpui::Context;
use std::ops::Range;

/// Pairs that close themselves when the opening one is typed.
/// A line moved out for a word like `end`: where it was, to put it back if the word turns
/// out to be another (`endpoint`).
pub(super) struct WordOutdent {
    line: usize,
    indent: String,
    /// The text's revision right after the move.
    revision: u64,
}

const PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\''), ('`', '`')];

impl Editor {
    /// The lines the selection touches (end exclusive). A selection ending at the very
    /// start of a line doesn't include that line, as in every editor.
    pub(super) fn selected_lines(&self) -> Range<usize> {
        let range = self.selection.range();
        let (start, _) = self.buffer.point(range.start);
        let (end, end_col) = self.buffer.point(range.end);
        let end = if end_col == 0 && end > start { end } else { end + 1 };
        start..end.min(self.buffer.len_lines()).max(start + 1)
    }

    pub(super) fn lines_char_range(&self, lines: &Range<usize>) -> Range<usize> {
        let start = self.buffer.line_to_char(lines.start);
        let end = if lines.end >= self.buffer.len_lines() {
            self.buffer.len_chars()
        } else {
            self.buffer.line_to_char(lines.end)
        };
        start..end
    }

    /// Replaces whole lines in one undo step, then places the selection with `place`,
    /// which gets the (line, column) of the old anchor and head.
    pub(super) fn rewrite_lines(
        &mut self,
        lines: Range<usize>,
        new_lines: Vec<String>,
        place: impl Fn((usize, usize)) -> (usize, usize),
        cx: &mut Context<Self>,
    ) {
        let range = self.lines_char_range(&lines);
        // The lines' own break, so a "\r\n" file stays one.
        let old = self.buffer.slice(range.clone());
        let ending = self.style.line_ending.text();
        let mut text = new_lines.join(ending);
        if old.ends_with(['\n', '\r']) {
            text.push_str(ending);
        }
        let anchor = self.buffer.point(self.selection.anchor);
        let head = self.buffer.point(self.selection.head);
        self.record_undo(EditKind::Other);
        self.buffer.replace(range, &text);
        let (al, ac) = place(anchor);
        let (hl, hc) = place(head);
        self.selection = Selection { anchor: self.buffer.offset(al, ac), head: self.buffer.offset(hl, hc) };
        self.goal_column = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }

    pub(super) fn line_texts(&self, lines: &Range<usize>) -> Vec<String> {
        lines.clone().map(|l| self.buffer.line_text(l)).collect()
    }

    // ---------- comments ----------

    /// ⌥⌘/: the selection wrapped in a block comment (`/* … */`), or unwrapped when it is
    /// one; with nothing selected, or over several lines, the whole lines.
    pub(super) fn toggle_block_comment(&mut self, cx: &mut Context<Self>) {
        let at = self.selection.head;
        let Some((_, block)) = self.comment_marks() else { return };
        let Some((open, close)) = block else {
            return self.show_notice(at, format!("{} has no block comments.", self.language_name()), cx);
        };
        let range = self.selection.range();
        let (first, _) = self.buffer.point(range.start);
        let (last, _) = self.buffer.point(range.end);
        if !range.is_empty() && first == last {
            let text = self.buffer.slice(range.clone());
            let inner =
                text.strip_prefix(open).and_then(|t| t.strip_suffix(close)).filter(|inner| !inner.contains(close));
            let new = match inner {
                Some(inner) => {
                    let inner = inner.strip_prefix(' ').unwrap_or(inner);
                    inner.strip_suffix(' ').unwrap_or(inner).to_string()
                }
                None => format!("{open} {text} {close}"),
            };
            let len = new.chars().count();
            self.edit(range.clone(), &new, EditKind::Other, cx);
            self.selection = Selection { anchor: range.start, head: range.start + len };
            return;
        }
        let lines = self.selected_lines();
        let texts = self.line_texts(&lines);
        let new_lines = toggle_block_comment(&texts, open, close);
        self.rewrite_lines(lines, new_lines, |p| p, cx);
    }

    /// How this file's comments are written: its line comment's mark and its block comment's
    /// ends, from its grammar or from the basic colouring (Swift, Ruby…). None for text.
    #[allow(clippy::type_complexity)]
    pub(super) fn comment_marks(&self) -> Option<(Option<&'static str>, Option<(&'static str, &'static str)>)> {
        match (self.language(), self.basic_syntax()) {
            (Some(language), _) => Some((language.line_comment, language.block_comment)),
            // Ruby's `=begin`/`=end` only work at a line's very start: not one to wrap with.
            (None, Some(basic)) => Some((
                basic.line_comment.first().copied(),
                basic.block_comment.filter(|(open, _)| !open.starts_with('=')),
            )),
            (None, None) => None,
        }
    }

    /// Comments the selected lines out, or back in when they all already are.
    pub(super) fn toggle_comment(&mut self, cx: &mut Context<Self>) {
        let Some((line_comment, block_comment)) = self.comment_marks() else { return };
        let lines = self.selected_lines();
        let texts = self.line_texts(&lines);
        if let Some(marker) = line_comment {
            let (new_lines, shift) = toggle_line_comments(&texts, marker);
            let start = lines.start;
            self.rewrite_lines(lines, new_lines, |(l, c)| (l, shift(l - start, c)), cx);
        } else if let Some((open, close)) = block_comment {
            let new_lines = toggle_block_comment(&texts, open, close);
            self.rewrite_lines(lines, new_lines, |p| p, cx);
        }
    }

    // ---------- indentation ----------

    pub(super) fn indent_lines(&mut self, cx: &mut Context<Self>) {
        let lines = self.selected_lines();
        let indented = lines.clone();
        let unit = self.style.indent.unit();
        let added = unit.chars().count();
        let new_lines = self
            .line_texts(&lines)
            .into_iter()
            .map(|t| if t.trim().is_empty() { t } else { format!("{unit}{t}") })
            .collect();
        // A selection ending at the start of the next line keeps that end where it is.
        self.rewrite_lines(lines, new_lines, move |(l, c)| (l, if indented.contains(&l) { c + added } else { c }), cx);
    }

    pub(super) fn outdent_lines(&mut self, cx: &mut Context<Self>) {
        let lines = self.selected_lines();
        let texts = self.line_texts(&lines);
        let removed: Vec<usize> = texts
            .iter()
            .map(|t| {
                if t.starts_with('\t') {
                    1
                } else {
                    t.chars().take(self.style.indent.width()).take_while(|c| *c == ' ').count()
                }
            })
            .collect();
        if removed.iter().all(|&r| r == 0) {
            return;
        }
        let new_lines = texts.iter().zip(&removed).map(|(t, &r)| t.chars().skip(r).collect()).collect();
        let start = lines.start;
        self.rewrite_lines(
            lines,
            new_lines,
            |(l, c)| (l, c.saturating_sub(removed.get(l - start).copied().unwrap_or(0))),
            cx,
        );
    }

    // ---------- moving, duplicating, deleting lines ----------

    pub(super) fn move_lines(&mut self, down: bool, cx: &mut Context<Self>) {
        let lines = self.selected_lines();
        // The empty "line" after a final line break isn't a line to swap with.
        let total = self.buffer.len_lines()
            - usize::from(self.buffer.len_lines() > 1 && self.buffer.line_len(self.buffer.len_lines() - 1) == 0);
        if lines.end > total {
            return;
        }
        if (!down && lines.start == 0) || (down && lines.end >= total) {
            return;
        }
        // Rewrite the selected lines together with the neighbour they swap with.
        let span = if down { lines.start..lines.end + 1 } else { lines.start - 1..lines.end };
        let mut texts = self.line_texts(&span);
        if down {
            let neighbour = texts.pop().unwrap_or_default();
            texts.insert(0, neighbour);
        } else {
            let neighbour = texts.remove(0);
            texts.push(neighbour);
        }
        let (first, last) = (span.start, span.end - 1);
        self.rewrite_lines(span.clone(), texts, |(l, c)| (if down { l + 1 } else { l - 1 }, c), cx);
        // A numbered item moved: the list counts in its new order.
        self.renumber_list(if down { first + 1 } else { first }, cx);
        // Bookmarks and breakpoints go with their lines; the neighbour's to the other end.
        self.marks_moved(
            span,
            |l| match (down, l) {
                (true, l) if l == last => first,
                (true, l) => l + 1,
                (false, l) if l == first => last,
                (false, l) => l - 1,
            },
            cx,
        );
    }

    pub(super) fn duplicate_lines(&mut self, down: bool, cx: &mut Context<Self>) {
        let lines = self.selected_lines();
        let texts = self.line_texts(&lines);
        let count = lines.len();
        let doubled = [texts.clone(), texts].concat();
        // Duplicating down moves the selection onto the copy; up keeps it on the original.
        self.rewrite_lines(lines, doubled, |(l, c)| (if down { l + count } else { l }, c), cx);
    }

    pub(super) fn delete_lines(&mut self, cx: &mut Context<Self>) {
        let lines = self.selected_lines();
        let (line, column) = self.caret_point();
        let mut range = self.lines_char_range(&lines);
        // On the last line, take the line break before it instead.
        if lines.end >= self.buffer.len_lines() && range.start > 0 {
            range.start -= 1;
        }
        self.record_undo(EditKind::Other);
        self.buffer.replace(range, "");
        let line = line.min(lines.start).min(self.buffer.len_lines() - 1);
        self.selection = Selection::caret(self.buffer.offset(line, column));
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        // A numbered item deleted: the ones after it count down.
        self.renumber_list(line, cx);
        self.touch(cx);
    }

    /// Selects the caret's whole line; repeating extends to the next line.
    pub(super) fn select_line(&mut self, cx: &mut Context<Self>) {
        let lines = self.selected_lines();
        let already_whole = self.selection.range() == self.lines_char_range(&lines) && !self.selection.is_empty();
        let lines = if already_whole { lines.start..(lines.end + 1).min(self.buffer.len_lines()) } else { lines };
        let range = self.lines_char_range(&lines);
        self.selection = Selection { anchor: range.start, head: range.end };
        self.touch(cx);
    }

    pub fn go_to_line(&mut self, line: usize, cx: &mut Context<Self>) {
        self.single_cursor();
        let line = line.saturating_sub(1).min(self.buffer.len_lines().saturating_sub(1));
        let text = self.buffer.line_text(line);
        let indent = text.len() - text.trim_start().len();
        self.selection = Selection::caret(self.buffer.offset(line, indent));
        self.goal_column = None;
        self.touch(cx);
    }

    // ---------- brackets and quotes ----------

    /// Handles typing a single bracket or quote. Returns true when it did something,
    /// so normal typing should be skipped.
    pub(super) fn type_pair_char(&mut self, c: char, cx: &mut Context<Self>) -> bool {
        // In Markdown, * _ ~ over a selection mark it up: *once* for italic, **twice** for bold.
        let marks = self.is_markdown() && !self.selection.is_empty() && matches!(c, '*' | '_' | '~');
        let pair = PAIRS.iter().copied().find(|(o, cl)| *o == c || *cl == c).or(marks.then_some((c, c)));
        let Some((open, close)) = pair else { return false };
        let caret = self.selection.head;
        let next = self.buffer.char_at(caret);
        let previous = caret.checked_sub(1).and_then(|i| self.buffer.char_at(i));
        let quote = open == close;

        // Typing the closing char right before an identical one just steps over it.
        if c == close && self.selection.is_empty() && next == Some(close) {
            self.selection = Selection::caret(caret + 1);
            self.touch(cx);
            return true;
        }
        if c != open {
            return false;
        }
        // With a selection, wrap it.
        if !self.selection.is_empty() {
            let range = self.selection.range();
            let inner = self.buffer.slice(range.clone());
            self.record_undo(EditKind::Other);
            self.buffer.replace(range.clone(), &format!("{open}{inner}{close}"));
            self.selection = Selection { anchor: range.start + 1, head: range.end + 1 };
            self.text_changed(cx);
            cx.emit(EditorEvent::Edited);
            self.touch(cx);
            return true;
        }
        // Only pair up where it's clearly wanted: before whitespace, a closer or the end
        // of the line; and for quotes, not right after a letter (don't, it's).
        let next_ok = next.is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | '}' | ',' | ';' | ':'));
        let previous_ok = !quote || previous.is_none_or(|p| !(p.is_alphanumeric() || p == '_' || p == open));
        if !(next_ok && previous_ok) {
            return false;
        }
        self.record_undo(EditKind::Typing);
        self.buffer.replace(caret..caret, &format!("{open}{close}"));
        self.selection = Selection::caret(caret + 1);
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
        true
    }

    /// Smart quotes and dashes in Markdown and text, when on (see `punctuation`). Returns
    /// true when it wrote the character itself.
    pub(super) fn type_smart_char(&mut self, c: char, cx: &mut Context<Self>) -> bool {
        use crate::punctuation::{QUOTES, Smart, smart};
        if !matches!(c, '"' | '\'' | '-')
            || !cx.global::<crate::settings::Settings>().smart_punctuation
            || !self.is_prose()
            || self.marked.is_some()
        {
            return false;
        }
        let range = self.selection.range();
        let (line, column) = self.buffer.point(range.start);
        if self.in_fence(line) {
            return false;
        }
        if self.in_front_matter(line) {
            return false;
        }
        let before: String = self.buffer.line_text(line).chars().take(column).collect();
        let smart = smart(c, &before, *QUOTES);
        // In an HTML comment still open from a line above, `--` is the start of its `-->`.
        if matches!(smart, Some(Smart::Join(_))) && self.in_open_comment(range.start) {
            return false;
        }
        match smart {
            // Over a selection, a quote wraps it.
            Some(Smart::Write(_)) if !range.is_empty() => {
                let (open, close) = if c == '"' { QUOTES.double } else { QUOTES.single };
                let inner = self.buffer.slice(range.clone());
                self.edit(range.clone(), &format!("{open}{inner}{close}"), EditKind::Other, cx);
                self.selection = Selection { anchor: range.start + 1, head: range.end + 1 };
                self.touch(cx);
            }
            // A closing quote typed right before the same one steps over it.
            Some(Smart::Write(w))
                if self.buffer.char_at(range.start) == Some(w) && w != QUOTES.double.0 && w != QUOTES.single.0 =>
            {
                self.selection = Selection::caret(range.start + 1);
                self.touch(cx);
            }
            Some(Smart::Write(w)) => self.edit(range, &w.to_string(), EditKind::Typing, cx),
            Some(Smart::Join(w)) if range.is_empty() => {
                self.edit(range.start - 1..range.start, &w.to_string(), EditKind::Typing, cx)
            }
            _ => return false,
        }
        true
    }

    /// Whether `line` is in the settings at the top of a Markdown file (`---` … `---`), or
    /// in ones still being written (no closing `---` yet, nor a blank line).
    fn in_front_matter(&self, line: usize) -> bool {
        if !self.is_markdown() || self.buffer.line_text(0).trim() != "---" {
            return false;
        }
        (1..line).all(|l| !matches!(self.buffer.line_text(l).trim(), "---" | ""))
    }

    /// Whether the text before `offset` has an HTML comment (`<!--`) not closed yet.
    fn in_open_comment(&self, offset: usize) -> bool {
        let before = self.buffer.rope().slice(..offset).to_string();
        before.rfind("<!--").is_some_and(|open| before.rfind("-->").is_none_or(|close| close < open))
    }

    /// Enter in a comment: the next line carries it on. Doc comments (`///`, `//!`) and block
    /// comments (` * `) always; a line comment (`//`, `#`) only when Enter splits it, text
    /// after the caret (Enter at its end starts code). Returns whether it did.
    pub(super) fn continue_comment(
        &mut self,
        range: Range<usize>,
        line: usize,
        col: usize,
        line_text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((line_comment, block_comment)) = self.comment_marks() else { return false };
        // Enter over a selection replaces it; only a plain Enter carries a comment on.
        if !range.is_empty() {
            return false;
        }
        let before: String = line_text.chars().take(col).collect();
        let after: String = line_text.chars().skip(col).collect();
        let Some(prefix) = comment_continuation(&before, &after, line_comment, block_comment) else {
            return false;
        };
        // Only in a comment, as the syntax has it (`* b` can be code).
        let byte = self.buffer.rope().char_to_byte(range.start);
        let line_bytes =
            self.buffer.line_to_byte(line)..self.buffer.line_to_byte((line + 1).min(self.buffer.len_lines()));
        self.highlight_bytes(line_bytes);
        if super::spelling::syntax_at(&self.spans, byte.saturating_sub(1)) != Some(crate::theme::Syntax::Comment) {
            return false;
        }
        // Splitting a comment: the words after the caret start right after the new prefix.
        let spaces = after.chars().take_while(|c| *c == ' ').count();
        let nl = self.style.line_ending.text();
        self.edit(range.start..range.end + spaces, &format!("{nl}{prefix}"), EditKind::Other, cx);
        true
    }

    /// ⇥ after an Emmet abbreviation (see `emmet`): its tags, as a snippet to fill in. In
    /// HTML, not in a script or a style sheet; in JSX, only in an element's content, where
    /// `a.b` can't be code. Returns whether it did.
    pub(super) fn expand_abbreviation(&mut self, cx: &mut Context<Self>) -> bool {
        let jsx = match self.language().map(|l| l.name) {
            Some("HTML") => false,
            Some("JavaScript" | "TSX") => true,
            _ => return false,
        };
        // One place to fill in at a time: not with several cursors.
        if !self.selection.is_empty() || self.multi_cursor() {
            return false;
        }
        let caret = self.selection.head;
        let (line, column) = self.buffer.point(caret);
        let text = self.buffer.line_text(line);
        let before: String = text.chars().take(column).collect();
        let indent = &text[..text.len() - text.trim_start().len()];
        let Some((start_byte, snippet)) = crate::emmet::expand_at_end(&before, indent, &self.style.indent.unit(), jsx)
        else {
            return false;
        };
        let start = caret - before[start_byte..].chars().count();
        // In JSX, the abbreviation is in an element's content when what's just before it is
        // (the abbreviation itself may not parse as JSX text: `{…}`, `>`).
        let in_place = if jsx { self.in_jsx_content(start) } else { self.in_html_text(caret) };
        if !in_place {
            return false;
        }
        let snippet = snippet.replace('\n', self.style.line_ending.text());
        let parsed = super::snippet::parse(&snippet);
        self.edit(start..caret, &parsed.text.clone(), EditKind::Other, cx);
        self.start_snippet(start, parsed, cx);
        cx.notify();
        true
    }

    /// `<` typed over selected text in HTML (or JSX content): the text wrapped in a tag, the
    /// name typed in both of its ends at once, ⇥ or Esc when it's done. Whole lines get the
    /// tag on lines of its own, and go in a level. Returns whether it did.
    pub(super) fn wrap_in_tag(&mut self, c: char, cx: &mut Context<Self>) -> bool {
        let range = self.selection.range();
        if c != '<' || range.is_empty() || self.multi_cursor() {
            return false;
        }
        // In JSX, where the selection starts: in an element's content (a selected `<li>…`
        // starts right after it).
        match self.language().map(|l| l.name) {
            Some("HTML") if self.in_html_text(range.start) => {}
            Some("JavaScript" | "TSX") if self.in_jsx_content(range.start) => {}
            _ => return false,
        }
        let (first, first_col) = self.buffer.point(range.start);
        let (last, last_col) = self.buffer.point(range.end);
        let first_text = self.buffer.line_text(first);
        let indent: String = first_text.chars().take_while(|c| c.is_whitespace()).collect();
        // Whole lines: from their indentation (or start) to the end of the last, or the
        // start of the line after it.
        let whole = last > first
            && first_col <= indent.chars().count()
            && (last_col == 0
                || last_col == self.buffer.line_text(last).trim_end_matches(['\n', '\r']).chars().count());
        let (start, snippet) = if whole {
            let end_line = if last_col == 0 { last - 1 } else { last };
            let unit = self.style.indent.unit();
            let lines: Vec<String> = (first..=end_line)
                .map(|l| {
                    let text = self.buffer.line_text(l);
                    let text = text.trim_end_matches(['\n', '\r']);
                    if text.trim().is_empty() {
                        String::new()
                    } else {
                        format!("{unit}{}", crate::emmet::literal(text))
                    }
                })
                .collect();
            let start = self.buffer.offset(first, 0);
            let end = self.buffer.offset(end_line, self.buffer.line_len(end_line));
            self.selection = Selection { anchor: start, head: end };
            let ending = self.style.line_ending.text();
            (start, format!("{indent}<$1>{ending}{}{ending}{indent}</$1>$0", lines.join(ending)))
        } else {
            let inner = crate::emmet::literal(&self.buffer.slice(range.clone()));
            (range.start, format!("<$1>{inner}</$1>$0"))
        };
        let parsed = super::snippet::parse(&snippet);
        let replace = self.selection.range();
        self.edit(replace, &parsed.text.clone(), EditKind::Other, cx);
        self.start_snippet(start, parsed, cx);
        cx.notify();
        true
    }

    /// Whether `at` is in an HTML page's text: not in a `<script>`, a `<style>` or a comment
    /// still open there (that's code, or nothing to expand).
    fn in_html_text(&self, at: usize) -> bool {
        let html = self.buffer.rope().slice(..at).to_string().to_lowercase();
        let open = |start: &str, end: &str| html.rfind(start).is_some_and(|at| html.rfind(end).is_none_or(|e| e < at));
        !(open("<script", "</script") || open("<style", "</style") || open("<!--", "-->"))
    }

    /// Whether the text just before `at` is in a JSX element's content (between its tags, not
    /// in a tag or a `{…}`), as the syntax tree has it.
    fn in_jsx_content(&mut self, at: usize) -> bool {
        let byte = self.buffer.rope().char_to_byte(at).saturating_sub(1);
        let Some(highlighter) = &mut self.highlighter else { return false };
        highlighter.sync(&self.buffer);
        let Some(tree) = highlighter.tree() else { return false };
        let mut node = tree.root_node().descendant_for_byte_range(byte, byte);
        while let Some(n) = node {
            match n.kind() {
                "jsx_element" | "jsx_fragment" => return true,
                "jsx_opening_element"
                | "jsx_closing_element"
                | "jsx_self_closing_element"
                | "jsx_expression"
                | "jsx_attribute" => return false,
                _ => node = n.parent(),
            }
        }
        false
    }

    /// A closing bracket typed first on its line: the line goes back to the indentation of
    /// the line its opening bracket is on.
    pub(super) fn outdent_closer(&mut self, cx: &mut Context<Self>) {
        let caret = self.selection.head;
        let (line, col) = self.buffer.point(caret);
        let text = self.buffer.line_text(line);
        let before: String = text.chars().take(col).collect();
        let Some(close) = before.chars().last().filter(|_| before.trim().chars().count() == 1) else { return };
        if self.comment_marks().is_none() || self.is_markdown() {
            return;
        }
        let open = match close {
            '}' => '{',
            ')' => '(',
            ']' => '[',
            _ => return,
        };
        // Its opening bracket: back from it, over the pairs inside, not counting brackets in
        // strings and comments (`'}'`), nor the one typed if it's in one itself.
        const LOOK_BACK: usize = 20_000;
        let from = caret.saturating_sub(LOOK_BACK);
        let rope = self.buffer.rope();
        let (from_byte, caret_byte) = (rope.char_to_byte(from), rope.char_to_byte(caret));
        self.highlight_bytes(from_byte..caret_byte);
        let quoted = |byte: usize, spans: &[crate::highlight::Span]| {
            matches!(
                super::spelling::syntax_at(spans, byte),
                Some(crate::theme::Syntax::String | crate::theme::Syntax::Comment)
            )
        };
        if quoted(caret_byte - close.len_utf8(), &self.spans) {
            return;
        }
        let rope = self.buffer.rope();
        let mut chars = rope.chars_at(caret - 1);
        let (mut depth, mut at, mut byte) = (1, caret - 1, caret_byte - close.len_utf8());
        while depth > 0 && at > from {
            let Some(c) = chars.prev() else { return };
            at -= 1;
            byte -= c.len_utf8();
            if quoted(byte, &self.spans) {
                continue;
            }
            if c == close {
                depth += 1;
            } else if c == open {
                depth -= 1;
            }
        }
        // Not found near enough: left where it is.
        if depth > 0 {
            return;
        }
        let open_line = self.buffer.point(at).0;
        if open_line == line {
            return;
        }
        let indent = |t: &str| t.chars().take_while(|c| *c == ' ' || *c == '\t').collect::<String>();
        let (wanted, current) = (indent(&self.buffer.line_text(open_line)), indent(&text));
        if wanted == current {
            return;
        }
        let start = self.buffer.line_to_char(line);
        self.edit(start..start + current.chars().count(), &wanted, EditKind::Typing, cx);
        self.selection = Selection::caret(start + wanted.chars().count() + 1);
    }

    /// Python: `else:`, `elif …:`, `except …:` or `finally:` just finished at the body's
    /// indentation goes out a level, to its `if` or `try`.
    pub(super) fn outdent_python_clause(&mut self, cx: &mut Context<Self>) {
        let caret = self.selection.head;
        let (line, col) = self.buffer.point(caret);
        let text = self.buffer.line_text(line);
        let before: String = text.chars().take(col).collect();
        let first_word =
            |t: &str| t.trim_start().split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or("").to_string();
        let word = first_word(&before);
        let after_ok = text.chars().skip(col).all(char::is_whitespace);
        // What it belongs to: `else` to an `if`, a loop or a `try`, `except` to a `try`…
        let openers: &[&str] = match word.as_str() {
            "else" => &["if", "elif", "for", "while", "try", "except"],
            "elif" => &["if", "elif"],
            "except" => &["try", "except"],
            "finally" => &["try", "except", "else"],
            _ => return,
        };
        if !before.ends_with(':') || !after_ok {
            return;
        }
        let indent_of = |t: &str| super::reindent::indent_columns(t, 4);
        let current = indent_of(&text);
        // Up to the nearest line it can belong to, no further in than it is.
        let opener = (0..line)
            .rev()
            .map(|l| self.buffer.line_text(l))
            .filter(|t| !t.trim().is_empty())
            .find(|t| indent_of(t) <= current && openers.contains(&first_word(t).as_str()));
        let Some(opener) = opener else { return };
        let wanted: String = opener.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let leading: String = text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        if wanted == leading {
            return;
        }
        let start = self.buffer.line_to_char(line);
        let (had, has) = (leading.chars().count(), wanted.chars().count());
        self.edit(start..start + had, &wanted, EditKind::Typing, cx);
        self.selection = Selection::caret(caret + has - had);
    }

    /// Ruby, Lua, the shell: `end`, `fi`, `else`… just typed first on its line goes back to
    /// the indentation of what it closes. Typed on into a longer word (`endpoint`, `file`),
    /// the line goes back where it was.
    pub(super) fn outdent_word(&mut self, cx: &mut Context<Self>) {
        let moved = self.word_outdent.take();
        let caret = self.selection.head;
        let (line, col) = self.buffer.point(caret);
        let text = self.buffer.line_text(line);
        let before: String = text.chars().take(col).collect();
        let word = before.trim_start();
        let is_word = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !is_word(word) || !text.chars().skip(col).all(char::is_whitespace) || !self.selection.is_empty() {
            return;
        }
        let language = self.language_name();
        let leading = &before[..before.len() - word.len()];
        let wanted = if crate::word_blocks::grew_from_word(language, word) {
            // Only right after the move, one letter later.
            match moved {
                Some(m) if m.line == line && m.revision + 1 == self.buffer.revision() => m.indent,
                _ => return,
            }
        } else if !crate::word_blocks::is_block_word(language, word) {
            return;
        } else {
            // Not in a string or a comment (Lua's `--[[ … ]]`).
            let rope = self.buffer.rope();
            let byte = rope.char_to_byte(caret - 1);
            self.highlight_bytes(byte..byte + 1);
            if matches!(
                super::spelling::syntax_at(&self.spans, byte),
                Some(crate::theme::Syntax::String | crate::theme::Syntax::Comment)
            ) {
                return;
            }
            let above: Vec<String> =
                (line.saturating_sub(2000)..line).rev().map(|l| self.buffer.line_text(l)).collect();
            match crate::word_blocks::closer_indent(language, &text, above.iter().map(String::as_str)) {
                Some(indent) => indent.to_string(),
                None => return,
            }
        };
        if wanted == leading {
            return;
        }
        let start = self.buffer.line_to_char(line);
        let (had, has) = (leading.chars().count(), wanted.chars().count());
        self.edit(start..start + had, &wanted, EditKind::Typing, cx);
        self.selection = Selection::caret(caret + has - had);
        self.word_outdent = Some(WordOutdent { line, indent: leading.to_string(), revision: self.buffer.revision() });
    }

    /// Backspace between an empty pair like `()` removes both.
    pub(super) fn empty_pair_around_caret(&self) -> bool {
        let caret = self.selection.head;
        let (Some(before), Some(after)) =
            (caret.checked_sub(1).and_then(|i| self.buffer.char_at(i)), self.buffer.char_at(caret))
        else {
            return false;
        };
        self.selection.is_empty() && PAIRS.contains(&(before, after))
    }

    /// The bracket next to the caret and the one that matches it, as char offsets.
    pub fn matching_brackets(&self) -> Option<(usize, usize)> {
        const LIMIT: usize = 20_000;
        let caret = self.selection.head;
        let at = |i: usize| self.buffer.char_at(i);
        let candidates = [caret, caret.wrapping_sub(1)];
        for i in candidates {
            let Some(c) = at(i) else { continue };
            if let Some(&(open, close)) =
                [('(', ')'), ('[', ']'), ('{', '}')].iter().find(|(o, cl)| *o == c || *cl == c)
            {
                // Walking the rope's chars in a row, not looking each one up: this runs every frame.
                let forward = c == open;
                let rope = self.buffer.rope();
                let mut chars = rope.chars_at(if forward { i + 1 } else { i });
                let mut depth = 1i32;
                let mut j = i;
                for _ in 0..LIMIT {
                    let next = if forward { chars.next() } else { chars.prev() };
                    let Some(x) = next else { break };
                    j = if forward { j + 1 } else { j - 1 };
                    if x == open {
                        depth += if forward { 1 } else { -1 };
                    } else if x == close {
                        depth += if forward { -1 } else { 1 };
                    }
                    if depth == 0 {
                        return Some((i, j));
                    }
                }
            }
        }
        None
    }
}

/// What the next line starts with when Enter is pressed after `before` (with `after` left
/// on the line), in a comment: the indentation and the comment's mark, or None.
fn comment_continuation(
    before: &str,
    after: &str,
    line_comment: Option<&str>,
    block_comment: Option<(&str, &str)>,
) -> Option<String> {
    let indent: String = before.chars().take_while(|c| c.is_whitespace()).collect();
    let text = &before[indent.len()..];
    // A block comment's lines: `/**` or `/*` opening it, `*` going on, not closed before the caret.
    if let Some((open, close)) = block_comment.filter(|(open, _)| *open == "/*")
        && !text.contains(close)
    {
        if text.starts_with(open) {
            return Some(format!("{indent} * "));
        }
        if text.starts_with('*') && !text.starts_with(close) {
            return Some(format!("{indent}* "));
        }
    }
    let mark = line_comment?;
    if !text.starts_with(mark) {
        return None;
    }
    // Rust's doc comments always go on; a plain comment only when split in two.
    let doc = mark == "//" && (text.starts_with("///") && !text.starts_with("////") || text.starts_with("//!"));
    let full = if doc { &text[..3] } else { mark };
    let written_space = text[full.len()..].starts_with(' ');
    if !doc && (after.trim().is_empty() || text[full.len()..].trim().is_empty()) {
        return None;
    }
    Some(format!("{indent}{full}{}", if written_space || !doc { " " } else { "" }))
}

/// Adds or removes `marker` on each line. Returns the new lines and how to move a
/// column on line `i` (relative to the first line) so the selection stays put.
fn toggle_line_comments(lines: &[String], marker: &str) -> (Vec<String>, impl Fn(usize, usize) -> usize + use<>) {
    let content: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    let all_commented = !content.is_empty() && content.iter().all(|l| l.trim_start().starts_with(marker));
    let indent = content.iter().map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let mut shifts: Vec<isize> = Vec::new();
    let new_lines = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                shifts.push(0);
                return line.clone();
            }
            if all_commented {
                let at = line.len() - line.trim_start().len();
                let rest = &line[at + marker.len()..];
                let removed = marker.len() + rest.starts_with(' ') as usize;
                shifts.push(-(removed as isize));
                format!("{}{}", &line[..at], rest.strip_prefix(' ').unwrap_or(rest))
            } else {
                shifts.push(marker.len() as isize + 1);
                // The common indentation, never cutting into a character (a non-breaking space...).
                let mut at = indent.min(line.len() - line.trim_start().len());
                while !line.is_char_boundary(at) {
                    at -= 1;
                }
                format!("{}{marker} {}", &line[..at], &line[at..])
            }
        })
        .collect();
    let shift = move |i: usize, col: usize| (col as isize + shifts.get(i).copied().unwrap_or(0)).max(0) as usize;
    (new_lines, shift)
}

fn toggle_block_comment(lines: &[String], open: &str, close: &str) -> Vec<String> {
    let first = lines.first().map(|l| l.trim_start()).unwrap_or("");
    let last = lines.last().map(|l| l.trim_end()).unwrap_or("");
    let mut lines = lines.to_vec();
    // One comment from the first line's start to the last line's end (not two that happen
    // to start and end there, `/* a */ f(); /* b */`).
    let all = lines.join("\n");
    let all = all.trim();
    let one = all.starts_with(open)
        && all.ends_with(close)
        && all.get(open.len()..all.len().saturating_sub(close.len())).is_some_and(|inside| !inside.contains(close));
    if first.starts_with(open) && last.ends_with(close) && one {
        let f = &mut lines[0];
        let at = f.find(open).unwrap_or(0);
        f.replace_range(at..at + open.len(), "");
        if f[at..].starts_with(' ') {
            f.remove(at);
        }
        let l = lines.last_mut().unwrap();
        if let Some(at) = l.rfind(close) {
            l.replace_range(at..at + close.len(), "");
            if l.ends_with(' ') {
                l.pop();
            }
        }
    } else {
        let indent = lines[0].len() - lines[0].trim_start().len();
        lines[0].insert_str(indent, &format!("{open} "));
        lines.last_mut().unwrap().push_str(&format!(" {close}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_carry_on_where_they_should() {
        let rust = (Some("//"), Some(("/*", "*/")));
        let next = |before: &str, after: &str| comment_continuation(before, after, rust.0, rust.1);
        assert_eq!(next("    /// Adds", "").as_deref(), Some("    /// "));
        assert_eq!(next("//! Crate", "").as_deref(), Some("//! "));
        assert_eq!(next("///", "").as_deref(), Some("///"), "a bare doc line: as it was written");
        assert_eq!(next("  /**", "").as_deref(), Some("   * "));
        assert_eq!(next("   * more", "").as_deref(), Some("   * "));
        assert_eq!(next("   */", ""), None, "the comment's end");
        assert_eq!(next("/* done */ x", ""), None);
        // A plain comment: only split in two.
        assert_eq!(next("// see", ""), None);
        assert_eq!(next("    // see the", " docs").as_deref(), Some("    // "));
        assert_eq!(next("let a = 1; // x", " y"), None, "code before it: not a comment line");
        assert_eq!(next("//// banner", " x").as_deref(), Some("// "));
        let python = |before: &str, after: &str| comment_continuation(before, after, Some("#"), None);
        assert_eq!(python("# note that", " this").as_deref(), Some("# "));
        assert_eq!(python("# note", ""), None);
    }

    #[test]
    fn two_comments_on_a_line_are_not_one() {
        let line = ["/* a */ f(); /* b */".to_string()];
        assert_eq!(toggle_block_comment(&line, "/*", "*/"), ["/* /* a */ f(); /* b */ */"]);
        let one = ["/* a".to_string(), "b */".to_string()];
        assert_eq!(toggle_block_comment(&one, "/*", "*/"), ["a", "b"]);
    }

    fn strings(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn line_comments_toggle_at_the_shared_indent() {
        let lines = strings(&["    let a = 1;", "", "        let b = 2;"]);
        let (commented, _) = toggle_line_comments(&lines, "//");
        assert_eq!(commented, strings(&["    // let a = 1;", "", "    //     let b = 2;"]));
        let (back, _) = toggle_line_comments(&commented, "//");
        assert_eq!(back, lines);
    }

    #[test]
    fn comment_shift_keeps_the_caret_on_the_same_code() {
        let (_, shift) = toggle_line_comments(&strings(&["x = 1"]), "#");
        assert_eq!(shift(0, 2), 4);
    }

    #[test]
    fn block_comments_wrap_and_unwrap() {
        let lines = strings(&["a { color: red; }"]);
        let wrapped = toggle_block_comment(&lines, "/*", "*/");
        assert_eq!(wrapped, strings(&["/* a { color: red; } */"]));
        assert_eq!(toggle_block_comment(&wrapped, "/*", "*/"), lines);
    }
}

/// Drives a real editor through GPUI's test harness.
#[cfg(test)]
mod editor_tests {
    use crate::buffer::Buffer;
    use crate::editor::{Editor, Selection};
    use gpui::{AppContext as _, TestAppContext};
    use std::path::PathBuf;

    fn editor(cx: &mut TestAppContext, text: &str, file: &str) -> gpui::Entity<Editor> {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
        });
        cx.new(|cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from(file)), cx))
    }

    fn text(cx: &mut TestAppContext, e: &gpui::Entity<Editor>) -> String {
        e.read_with(cx, |e, _| e.buffer.to_string())
    }

    fn select(cx: &mut TestAppContext, e: &gpui::Entity<Editor>, anchor: usize, head: usize) {
        e.update(cx, |e, _| e.selection = Selection { anchor, head });
    }

    #[gpui::test]
    fn editing_keeps_to_the_file_s_own_style(cx: &mut TestAppContext) {
        // A Go file indents with tabs; a 2-space file with 2 spaces.
        let e = editor(cx, "func f() {\n}\n", "main.go");
        select(cx, &e, 10, 10);
        e.update(cx, |e, cx| e.tab_key(cx));
        assert_eq!(text(cx, &e), "func f() {\t\n}\n");
        let e = editor(cx, "a {\n  b {\n    c\n  }\n}\n", "x.js");
        select(cx, &e, 0, 0);
        e.update(cx, |e, cx| e.tab_key(cx));
        assert_eq!(text(cx, &e), "  a {\n  b {\n    c\n  }\n}\n");
        // A Windows file gets Windows line breaks.
        let e = editor(cx, "one\r\ntwo\r\n", "x.txt");
        select(cx, &e, 3, 3);
        e.update(cx, |e, cx| {
            let at = e.selection.head;
            let nl = e.style.line_ending.text();
            e.edit(at..at, nl, super::super::EditKind::Other, cx);
        });
        assert_eq!(text(cx, &e), "one\r\n\r\ntwo\r\n");
    }

    #[gpui::test]
    fn saving_tidies_as_the_file_asks(cx: &mut TestAppContext) {
        let e = editor(cx, "a  \r\nb\nc", "x.txt");
        e.update(cx, |e, cx| {
            e.style.trim_trailing = true;
            e.style.final_newline = Some(true);
            e.style.line_ending = crate::file_style::LineEnding::Crlf;
            e.tidy_for_save(cx);
        });
        assert_eq!(text(cx, &e), "a\r\nb\r\nc\r\n");
        // One undo step takes it all back.
        e.update(cx, |e, cx| e.step_history(true, cx));
        assert_eq!(text(cx, &e), "a  \r\nb\nc");
    }

    #[gpui::test]
    fn folds_follow_edits_and_open_for_the_caret(cx: &mut TestAppContext) {
        use super::super::EditKind;
        let e = editor(cx, "fn a() {\n    one();\n    two();\n}\nfn b() {}\n", "x.rs");
        e.update(cx, |e, cx| {
            e.toggle_fold(0, cx);
            assert!(e.is_folded(0));
            e.wrap.update(&e.buffer, None, &[]);
            assert_eq!(e.wrap.rows(), 4); // 6 lines, 2 hidden
            // A line added above moves the fold down with its code.
            e.edit(0..0, "// a\n", EditKind::Other, cx);
            assert!(e.is_folded(1) && !e.is_folded(0));
            // Typing on the folded line itself keeps it folded.
            let at = e.buffer.offset(1, 4);
            e.edit(at..at, "x", EditKind::Typing, cx);
            assert!(e.is_folded(1));
            // The caret going inside (a search, a jump) opens it.
            e.selection = Selection::caret(e.buffer.offset(2, 2));
            e.touch(cx);
            assert!(!e.is_folded(1));
        });
    }

    #[gpui::test]
    fn undo_takes_back_a_word_at_a_time(cx: &mut TestAppContext) {
        use super::super::EditKind;
        let e = editor(cx, "\n", "x.rs");
        let type_at = |cx: &mut TestAppContext, text: &str| {
            for c in text.chars() {
                e.update(cx, |e, cx| {
                    let at = e.selection.head;
                    e.edit(at..at, &c.to_string(), EditKind::Typing, cx)
                });
            }
        };
        type_at(cx, "let total = 1;");
        let undo = |cx: &mut TestAppContext| e.update(cx, |e, cx| e.step_history(true, cx));
        undo(cx);
        assert_eq!(text(cx, &e), "let total = \n");
        // A symbol goes with the word before it.
        undo(cx);
        assert_eq!(text(cx, &e), "let \n");
        // Typing somewhere else is its own step, even right away.
        select(cx, &e, 0, 0);
        type_at(cx, "a");
        select(cx, &e, 5, 5);
        type_at(cx, "b");
        undo(cx);
        assert_eq!(text(cx, &e), "alet \n");
    }

    #[gpui::test]
    fn toggles_comments_over_a_selection(cx: &mut TestAppContext) {
        let e = editor(cx, "fn a() {\n    one();\n    two();\n}\n", "x.rs");
        select(cx, &e, 9, 30); // lines 2-3
        e.update(cx, |e, cx| e.toggle_comment(cx));
        assert_eq!(text(cx, &e), "fn a() {\n    // one();\n    // two();\n}\n");
        e.update(cx, |e, cx| e.toggle_comment(cx));
        assert_eq!(text(cx, &e), "fn a() {\n    one();\n    two();\n}\n");
    }

    #[gpui::test]
    fn moves_duplicates_and_deletes_lines(cx: &mut TestAppContext) {
        let e = editor(cx, "a\nb\nc\n", "x.py");
        select(cx, &e, 0, 0);
        e.update(cx, |e, cx| e.move_lines(true, cx));
        assert_eq!(text(cx, &e), "b\na\nc\n");
        assert_eq!(e.read_with(cx, |e, _| e.caret_point()), (1, 0));
        e.update(cx, |e, cx| e.duplicate_lines(true, cx));
        assert_eq!(text(cx, &e), "b\na\na\nc\n");
        e.update(cx, |e, cx| e.delete_lines(cx));
        assert_eq!(text(cx, &e), "b\na\nc\n");
        // Moving the last line down does nothing.
        select(cx, &e, 6, 6);
        e.update(cx, |e, cx| e.move_lines(true, cx));
        assert_eq!(text(cx, &e), "b\na\nc\n");
    }

    #[gpui::test]
    fn indents_and_outdents_selected_lines(cx: &mut TestAppContext) {
        let e = editor(cx, "x = 1\n\ny = 2\n", "x.py");
        select(cx, &e, 0, 12);
        e.update(cx, |e, cx| e.indent_lines(cx));
        assert_eq!(text(cx, &e), "    x = 1\n\n    y = 2\n");
        e.update(cx, |e, cx| e.outdent_lines(cx));
        assert_eq!(text(cx, &e), "x = 1\n\ny = 2\n");
    }

    #[gpui::test]
    fn tab_indents_any_selection_and_shift_tab_outdents(cx: &mut TestAppContext) {
        let e = editor(cx, "x = 1\ny = 2\n", "x.py");
        // Part of a single line selected: Tab indents that line instead of replacing the text.
        select(cx, &e, 0, 1);
        e.update(cx, |e, cx| e.tab_key(cx));
        assert_eq!(text(cx, &e), "    x = 1\ny = 2\n");
        // Shift+Tab outdents the caret's line even with nothing selected.
        select(cx, &e, 6, 6);
        e.update(cx, |e, cx| e.outdent_lines(cx));
        assert_eq!(text(cx, &e), "x = 1\ny = 2\n");
        // With no selection, Tab still inserts spaces at the caret.
        select(cx, &e, 1, 1);
        e.update(cx, |e, cx| e.tab_key(cx));
        assert_eq!(text(cx, &e), "x    = 1\ny = 2\n");
    }

    #[gpui::test]
    fn up_and_down_move_by_wrapped_rows(cx: &mut TestAppContext) {
        let e = editor(cx, &format!("{}\nend\n", "word ".repeat(10)), "x.md");
        e.update(cx, |e, cx| {
            e.wrap.update(&e.buffer, Some(20), &[]);
            e.selection = Selection::caret(2);
            e.move_vertically(1, false, cx);
            // Same column, one row down, still on the first line.
            assert_eq!(e.caret_point(), (0, 22));
            e.move_vertically(1, false, cx);
            e.move_vertically(1, false, cx);
            assert_eq!(e.caret_point(), (1, 2));
            e.move_vertically(-1, false, cx);
            assert_eq!(e.caret_point(), (0, 42));
        });
    }

    #[gpui::test]
    fn the_caret_and_view_come_back_as_they_were(cx: &mut TestAppContext) {
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let e = editor(cx, &text, "x.txt");
        e.update(cx, |e, cx| e.restore_view(120, 3, 100, cx));
        assert_eq!(e.read_with(cx, |e, _| e.view_state()), (120, 3, 100));
    }

    #[gpui::test]
    fn brackets_and_quotes_close_themselves(cx: &mut TestAppContext) {
        let e = editor(cx, "", "x.rs");
        e.update(cx, |e, cx| assert!(e.type_pair_char('(', cx)));
        assert_eq!(text(cx, &e), "()");
        // Typing the closer steps over the one already there.
        e.update(cx, |e, cx| assert!(e.type_pair_char(')', cx)));
        assert_eq!((text(cx, &e), e.read_with(cx, |e, _| e.selection.head)), ("()".into(), 2));
        // No pairing for an apostrophe inside a word.
        let e = editor(cx, "don", "x.rs");
        select(cx, &e, 3, 3);
        e.update(cx, |e, cx| assert!(!e.type_pair_char('\'', cx)));
        // A selection gets wrapped.
        let e = editor(cx, "value", "x.rs");
        select(cx, &e, 0, 5);
        e.update(cx, |e, cx| assert!(e.type_pair_char('"', cx)));
        assert_eq!(text(cx, &e), "\"value\"");
    }

    #[gpui::test]
    fn finds_the_matching_bracket(cx: &mut TestAppContext) {
        let e = editor(cx, "f(a, [b], c)", "x.rs");
        select(cx, &e, 1, 1);
        assert_eq!(e.read_with(cx, |e, _| e.matching_brackets()), Some((1, 11)));
        select(cx, &e, 8, 8);
        assert_eq!(e.read_with(cx, |e, _| e.matching_brackets()), Some((7, 5)));
    }

    /// ⇥ after `ul>li*2` in HTML: the tags, the first place to fill in selected, ⇥ to the
    /// next; a plain word stays a word; in a script, ⇥ is a tab.
    #[gpui::test]
    fn tab_expands_abbreviations_in_html(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let typed = |cx: &mut TestAppContext, start: &str, keys: &str| {
            let start = start.to_string();
            let (e, cx) = cx
                .add_window_view(|_, cx| Editor::new(Buffer::from_text(&start), Some(PathBuf::from("page.html")), cx));
            e.update_in(cx, |e, window, cx| {
                window.focus(&gpui::Focusable::focus_handle(e, cx));
                e.selection = Selection::caret(e.buffer.len_chars());
            });
            cx.simulate_input(keys);
            cx.simulate_keystrokes("tab");
            cx.simulate_input("One");
            cx.simulate_keystrokes("tab");
            cx.simulate_input("Two");
            e.read_with(cx, |e, _| e.buffer.to_string())
        };
        assert_eq!(
            typed(cx, "<body>\n  <p>Hi</p>\n  <div>\n    <p>There</p>\n  </div>\n  ", "ul>li*2"),
            "<body>\n  <p>Hi</p>\n  <div>\n    <p>There</p>\n  </div>\n  <ul>\n    <li>One</li>\n    <li>Two</li>\n  </ul>"
        );
        // Windows line breaks stay Windows ones.
        assert_eq!(typed(cx, "<body>\r\n", "ul>li"), "<body>\r\n<ul>\r\n    <li>One</li>\r\n</ul>Two");
        // A word, or code in a script: ⇥ is a tab, no tags.
        assert_eq!(typed(cx, "", "hello"), "hello   One Two");
        assert_eq!(typed(cx, "<script>\n", "ul>li"), "<script>\nul>li   One Two");
    }

    /// A closing bracket typed first on its line goes back to its opening line's indentation;
    /// in Python, a finished block steps out after `return`, and `else:` goes back to its `if`.
    #[gpui::test]
    fn closing_lines_step_back_out(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let typed = |cx: &mut TestAppContext, file: &str, text: &str, keys: &[&str]| {
            let (text, file) = (text.to_string(), PathBuf::from(file));
            let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(file), cx));
            e.update_in(cx, |e, window, cx| {
                window.focus(&gpui::Focusable::focus_handle(e, cx));
                e.selection = Selection::caret(e.buffer.len_chars());
            });
            for key in keys {
                match *key {
                    "enter" => cx.simulate_keystrokes("enter"),
                    text => cx.simulate_input(text),
                }
            }
            e.read_with(cx, |e, _| (e.buffer.to_string(), e.caret_point()))
        };
        let (text, caret) = typed(cx, "a.rs", "fn f() {\n    let a = vec![\n        1,\n        ", &["]"]);
        assert_eq!(text, "fn f() {\n    let a = vec![\n        1,\n    ]");
        assert_eq!(caret, (3, 5));
        let (text, _) = typed(cx, "a.rs", "fn f() {\n    x();\n    ", &["}"]);
        assert_eq!(text, "fn f() {\n    x();\n}");
        let (text, _) = typed(cx, "a.py", "def f(a):\n    if a:\n        return 1", &["enter", "x"]);
        assert_eq!(text, "def f(a):\n    if a:\n        return 1\n    x");
        let (text, caret) = typed(cx, "a.py", "def f(a):\n    if a:\n        g()\n        else", &[":"]);
        assert_eq!(text, "def f(a):\n    if a:\n        g()\n    else:");
        assert_eq!(caret, (3, 9));
        // Already out: left as it is.
        let (text, _) = typed(cx, "a.py", "if a:\n    g()\nelse", &[":"]);
        assert_eq!(text, "if a:\n    g()\nelse:");
        // After a `return` stepped out, `else:` stays with its `if` (not out a second time).
        let (text, _) = typed(cx, "a.py", "def f(a):\n    if a:\n        return 1", &["enter", "else:"]);
        assert_eq!(text, "def f(a):\n    if a:\n        return 1\n    else:");
        // A name that starts like one (`finally_cb:`) is left alone.
        let (text, _) = typed(cx, "a.py", "class A:\n    x: int\n    finally_cb", &[":"]);
        assert_eq!(text, "class A:\n    x: int\n    finally_cb:");
        // `return (` then Enter: inside the brackets, a level in.
        let (text, _) = typed(cx, "a.py", "def f():\n    return (", &["enter", "x"]);
        assert_eq!(text, "def f():\n    return (\n        x");
        // A `}` in a string isn't counted: the closing one lines up with its own `{`.
        let (text, _) = typed(cx, "a.rs", "impl A {\n    fn f(c: char) -> bool {\n        c == '}'\n        ", &["}"]);
        assert_eq!(text, "impl A {\n    fn f(c: char) -> bool {\n        c == '}'\n    }");
        // Unbalanced (no opening one): left where it is.
        let (text, _) = typed(cx, "a.rs", "fn f() {}\n    ", &["}"]);
        assert_eq!(text, "fn f() {}\n    }");
        // Enter between a tag and its closing one opens it up, as between braces.
        let at = |cx: &mut TestAppContext, file: &str, text: &str, col: usize| {
            let (text, file) = (text.to_string(), PathBuf::from(file));
            let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(file), cx));
            e.update_in(cx, |e, window, cx| {
                window.focus(&gpui::Focusable::focus_handle(e, cx));
                e.selection = Selection::caret(col);
            });
            cx.simulate_keystrokes("enter");
            cx.simulate_input("x");
            e.read_with(cx, |e, _| e.buffer.to_string())
        };
        assert_eq!(at(cx, "a.html", "  <div class=\"a\"></div>", 17), "  <div class=\"a\">\n    x\n  </div>");
        assert_eq!(at(cx, "a.jsx", "<ul></ul>", 4), "<ul>\n    x\n</ul>");
        // Not after a closing or self-closing tag.
        assert_eq!(at(cx, "a.html", "<p></p></div>", 7), "<p></p>\nx</div>");
    }

    /// Ruby, Lua, the shell and YAML: Enter after `do`, `then` or `key:` goes in a level;
    /// `end`, `fi` or `else` typed first on a line goes back to what it closes, and back in
    /// if it was the start of another word.
    #[gpui::test]
    fn word_blocks_step_in_and_out(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        // Keys one by one, as typed.
        let typed = |cx: &mut TestAppContext, file: &str, text: &str, keys: &str| {
            let (text, file) = (text.to_string(), PathBuf::from(file));
            let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(file), cx));
            e.update_in(cx, |e, window, cx| {
                window.focus(&gpui::Focusable::focus_handle(e, cx));
                e.selection = Selection::caret(e.buffer.len_chars());
            });
            for c in keys.chars() {
                match c {
                    '\n' => cx.simulate_keystrokes("enter"),
                    c => cx.simulate_input(&c.to_string()),
                }
            }
            e.read_with(cx, |e, _| e.buffer.to_string())
        };
        let ruby = "class A\n  def b\n    x\n  end\nend\n\n";
        assert_eq!(
            typed(cx, "a.rb", ruby, "def f(xs)\nxs.each do |x|\nputs x\nend\nend"),
            format!("{ruby}def f(xs)\n  xs.each do |x|\n    puts x\n  end\nend")
        );
        assert_eq!(typed(cx, "a.rb", ruby, "if a\nb\nelse\nc\nend"), format!("{ruby}if a\n  b\nelse\n  c\nend"));
        // A word that starts like one goes back in.
        assert_eq!(typed(cx, "a.rb", "def f\n  ", "ending = 1"), "def f\n  ending = 1");
        assert_eq!(typed(cx, "a.sh", "if a; then\n  ", "file=x"), "if a; then\n  file=x");
        assert_eq!(
            typed(cx, "a.lua", "local M = {}\n\n", "function M.f(a)\nif a then\nreturn 1\nelseif b then\nend\nend"),
            "local M = {}\n\nfunction M.f(a)\n    if a then\n        return 1\n    elseif b then\n    end\nend"
        );
        assert_eq!(
            typed(cx, "a.sh", "#!/bin/sh\n", "for f in *; do\necho $f\ndone"),
            "#!/bin/sh\nfor f in *; do\n    echo $f\ndone"
        );
        assert_eq!(
            typed(cx, "a.yml", "on:\n  push:\n", "jobs:\nbuild:\nsteps:\n- name: a\nrun: b"),
            "on:\n  push:\njobs:\n  build:\n    steps:\n      - name: a\n        run: b"
        );
        // In a comment: left alone.
        assert_eq!(typed(cx, "a.lua", "function f()\n  --[[ notes\n  ", "end"), "function f()\n  --[[ notes\n  end");
    }

    /// Enter in a doc comment carries it on, splitting a plain comment too; code that looks
    /// like a comment line (`* c`) doesn't.
    #[gpui::test]
    fn enter_carries_comments_on(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let typed = |cx: &mut TestAppContext, text: &str, line: usize, col: usize, keys: &str| {
            let text = text.to_string();
            let (e, cx) =
                cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(PathBuf::from("a.rs")), cx));
            e.update_in(cx, |e, window, cx| {
                window.focus(&gpui::Focusable::focus_handle(e, cx));
                e.selection = Selection::caret(e.buffer.offset(line, col));
            });
            cx.run_until_parked();
            cx.simulate_keystrokes("enter");
            cx.simulate_input(keys);
            e.read_with(cx, |e, _| e.buffer.to_string())
        };
        assert_eq!(typed(cx, "/// Adds one.\nfn f() {}\n", 0, 13, "More."), "/// Adds one.\n/// More.\nfn f() {}\n");
        assert_eq!(
            typed(cx, "fn f() {\n    // see the docs\n}\n", 1, 14, ""),
            "fn f() {\n    // see the\n    // docs\n}\n"
        );
        assert_eq!(typed(cx, "// note\n", 0, 7, "x"), "// note\nx\n", "at its end: code goes on");
        // Enter over a selection in a comment: the selection replaced, nothing else taken.
        let (e, cx2) = cx.add_window_view(|_, cx| {
            Editor::new(Buffer::from_text("/// foo bar\nfn f() {}\n"), Some("b.rs".into()), cx)
        });
        e.update_in(cx2, |e, window, cx| {
            window.focus(&gpui::Focusable::focus_handle(e, cx));
            e.selection = Selection { anchor: 7, head: 11 };
        });
        cx2.simulate_keystrokes("enter");
        assert!(e.read_with(cx2, |e, _| e.buffer.to_string()).ends_with("\nfn f() {}\n"), "the next line kept");
        assert!(super::comment_continuation("<!-- TODO", "", None, Some(("<!--", "-->"))).is_none(), "not HTML's");
        assert_eq!(
            typed(cx, "fn f() {\n    let a = b\n        * c\n}\n", 2, 11, "d"),
            "fn f() {\n    let a = b\n        * c\n        d\n}\n",
            "code, not a comment"
        );
    }

    /// Numbered lists count on after Enter in the middle, a line moved, a line deleted; one
    /// undo takes back the edit and the numbers together.
    #[gpui::test]
    fn numbered_lists_stay_in_order(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let text = "# Steps\n\n1. one\n2. two\n3. three\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("a.md")), cx));
        e.update_in(cx, |e, window, cx| {
            window.focus(&gpui::Focusable::focus_handle(e, cx));
            e.selection = Selection::caret(e.buffer.offset(2, 6));
        });
        cx.simulate_keystrokes("enter");
        cx.simulate_input("new");
        let now = |cx: &mut gpui::VisualTestContext| e.read_with(cx, |e, _| e.buffer.to_string());
        assert_eq!(now(cx), "# Steps\n\n1. one\n2. new\n3. two\n4. three\n");
        e.update(cx, |e, _| assert_eq!(e.caret_point(), (3, 6)));
        // Moved down past "two": each keeps its place in the count.
        e.update(cx, |e, cx| e.move_lines(true, cx));
        assert_eq!(now(cx), "# Steps\n\n1. one\n2. two\n3. new\n4. three\n");
        // Deleted: the rest count down.
        e.update(cx, |e, cx| e.delete_lines(cx));
        assert_eq!(now(cx), "# Steps\n\n1. one\n2. two\n3. three\n");
        // One undo: the line back, numbers and all.
        e.update(cx, |e, cx| e.step_history(true, cx));
        assert_eq!(now(cx), "# Steps\n\n1. one\n2. two\n3. new\n4. three\n");
        // Two lines selected and moved: still selected, ready to move again.
        e.update(cx, |e, _| e.selection = Selection { anchor: e.buffer.offset(2, 0), head: e.buffer.offset(3, 6) });
        e.update(cx, |e, cx| e.move_lines(true, cx));
        assert_eq!(now(cx), "# Steps\n\n1. new\n2. one\n3. two\n4. three\n");
        e.update(cx, |e, _| assert_eq!(e.buffer.slice(e.selection.range()), "2. one\n3. two"));
        // A list numbered all the same goes on that way.
        let (lazy, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text("1. a\n1. b"), Some("l.md".into()), cx));
        lazy.update_in(cx, |e, window, cx| {
            window.focus(&gpui::Focusable::focus_handle(e, cx));
            e.selection = Selection::caret(e.buffer.len_chars());
        });
        cx.simulate_keystrokes("enter");
        assert_eq!(lazy.read_with(cx, |e, _| e.buffer.to_string()), "1. a\n1. b\n1. ");
    }

    /// `<` over selected text wraps it in a tag: the name goes in both ends as it's typed;
    /// over whole lines, the tag takes lines of its own. Not in code.
    #[gpui::test]
    fn typing_a_bracket_over_a_selection_wraps_it_in_a_tag(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let wrapped = |cx: &mut TestAppContext, file: &str, text: &str, selected: std::ops::Range<usize>| {
            let (text, file) = (text.to_string(), PathBuf::from(file));
            let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(file), cx));
            e.update_in(cx, |e, window, cx| {
                window.focus(&gpui::Focusable::focus_handle(e, cx));
                e.selection = Selection { anchor: selected.start, head: selected.end };
            });
            cx.simulate_input("<strong");
            cx.simulate_keystrokes("tab");
            cx.simulate_input("!");
            e.read_with(cx, |e, _| e.buffer.to_string())
        };
        assert_eq!(
            wrapped(cx, "a.html", "<p>Very important</p>\n", 8..17),
            "<p>Very <strong>important</strong>!</p>\n"
        );
        assert_eq!(
            wrapped(cx, "a.html", "<ul>\n  <li>a</li>\n  <li>b</li>\n</ul>\n", 5..30),
            "<ul>\n  <strong>\n    <li>a</li>\n    <li>b</li>\n  </strong>!\n</ul>\n"
        );
        let jsx = "const P = () => <p>Hello there</p>;\n";
        assert_eq!(wrapped(cx, "P.jsx", jsx, 25..30), "const P = () => <p>Hello <strong>there</strong>!</p>;\n");
        // A whole element selected in JSX: wrapped too.
        let list = "const L = () => (\n  <ul>\n    <li>a</li>\n  </ul>\n);\n";
        assert_eq!(
            wrapped(cx, "L.jsx", list, 29..39),
            "const L = () => (\n  <ul>\n    <strong><li>a</li></strong>!\n  </ul>\n);\n"
        );
        // In code, and in a script, < is just typed over it.
        assert_eq!(wrapped(cx, "a.rs", "let a = b;\n", 8..9), "let a = <strong !;\n");
        let script = wrapped(cx, "a.html", "<script>\na > b\n", 11..12);
        assert!(script.starts_with("<script>\na <strong") && !script.contains("</strong>"), "{script}");
    }

    /// In JSX, ⇥ expands inside an element's content only: JSX's names there; plain code
    /// (`user.name`) and a `{…}` stay as they are.
    #[gpui::test]
    fn tab_expands_abbreviations_in_jsx_content(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let typed = |cx: &mut TestAppContext, file: &str, start: &str, keys: &str, end: &str| {
            let text = format!("{start}{end}");
            let caret = start.chars().count();
            let file = PathBuf::from(file);
            let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(file), cx));
            e.update_in(cx, |e, window, cx| {
                window.focus(&gpui::Focusable::focus_handle(e, cx));
                e.selection = Selection::caret(caret);
            });
            cx.simulate_input(keys);
            cx.simulate_keystrokes("tab");
            e.read_with(cx, |e, _| e.buffer.to_string())
        };
        let page = "const Page = () => (\n  <main>\n    ";
        let close = "\n  </main>\n);\n";
        assert_eq!(typed(cx, "Page.jsx", page, ".card", close), format!("{page}<div className=\"card\"></div>{close}"));
        assert_eq!(typed(cx, "Page.tsx", page, "img", close), format!("{page}<img src=\"\" alt=\"\" />{close}"));
        // Code, and an expression inside the element: ⇥ is a tab.
        let code = typed(cx, "user.tsx", "const name = ", "user.name", ";\n");
        assert!(!code.contains('<'), "{code}");
        let inside = typed(cx, "Page.jsx", "const P = () => <p>{", "a.b", "}</p>;\n");
        assert!(!inside.contains("<div"), "{inside}");
        // Text in braces and children, as in HTML.
        assert_eq!(typed(cx, "Page.jsx", page, "p{Hello}", close), format!("{page}<p>Hello</p>{close}"));
        assert_eq!(
            typed(cx, "Page.jsx", page, "ul>li", close),
            format!("{page}<ul>\n      <li></li>\n    </ul>{close}")
        );
    }

    /// Typed one key at a time, smart quotes and dashes on: curled in Markdown prose, left
    /// alone in its code and in code files, and off by default.
    #[gpui::test]
    fn smart_quotes_and_dashes_when_writing(cx: &mut TestAppContext) {
        use crate::punctuation::QUOTES;
        use gpui::EntityInputHandler as _;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings { smart_punctuation: true, ..Default::default() });
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let typed = |cx: &mut TestAppContext, file: &str, start: &str, keys: &str| {
            let start = start.to_string();
            let file = PathBuf::from(file);
            let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&start), Some(file), cx));
            e.update_in(cx, |e, window, cx| {
                e.selection = Selection::caret(e.buffer.len_chars());
                for key in keys.chars() {
                    e.replace_text_in_range(None, &key.to_string(), window, cx);
                }
                e.buffer.to_string()
            })
        };
        let (open, close) = QUOTES.double;
        let (_, apostrophe) = QUOTES.single;
        assert_eq!(
            typed(cx, "notes.md", "", "Say \"it's here\" -- now"),
            format!("Say {open}it{apostrophe}s here{close} — now")
        );
        // Code stays as typed: in ticks, in a fence, in a code file.
        assert_eq!(typed(cx, "notes.md", "", "`a--b \"c\"`"), "`a--b \"c\"`");
        assert_eq!(typed(cx, "notes.md", "```\n", "x--\"y"), "```\nx--\"y\"");
        assert_eq!(typed(cx, "main.rs", "", "a--b"), "a--b");
        // Front matter is settings; `--` in a comment open from above is its end.
        assert_eq!(typed(cx, "notes.md", "---\ntitle: ", "\"Day 1\""), "---\ntitle: \"Day 1\"");
        assert_eq!(typed(cx, "notes.md", "<!--\nold draft ", "-->"), "<!--\nold draft -->");
        assert_eq!(typed(cx, "notes.md", "<!-- a -->\nso ", "--"), "<!-- a -->\nso —");
        // Over a selection, a quote wraps it.
        let (e, cx2) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text("a word"), Some("x.txt".into()), cx));
        e.update_in(cx2, |e, window, cx| {
            e.selection = Selection { anchor: 2, head: 6 };
            e.replace_text_in_range(None, "\"", window, cx);
            assert_eq!(e.buffer.to_string(), format!("a {open}word{close}"));
            assert_eq!(e.selection.range(), 3..7);
        });
        // Off, quotes are typed as they are.
        cx.update(|cx| cx.set_global(crate::settings::Settings::default()));
        assert_eq!(typed(cx, "notes.md", "", "\"x\""), "\"x\"");
    }
}

#[cfg(test)]
mod basic_colouring_tests {
    use crate::buffer::Buffer;
    use crate::editor::{Editor, Selection};
    use gpui::{AppContext as _, TestAppContext};
    use std::path::PathBuf;

    /// A Swift file without a grammar: named Swift, coloured, ⌘/ comments it with `//`.
    #[gpui::test]
    fn languages_without_a_grammar_still_read_as_code(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
        });
        let text = "let n = 42\n";
        let e = cx.new(|cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("a.swift")), cx));
        e.update(cx, |e, cx| {
            assert_eq!(e.language_name(), "Swift");
            e.highlight_bytes(0..text.len());
            assert!(e.spans.iter().any(|(r, s)| *r == (0..3) && *s == crate::theme::Syntax::Keyword));
            e.selection = Selection::caret(0);
            e.toggle_comment(cx);
            assert_eq!(e.buffer.to_string(), "// let n = 42\n");
        });
        // XML has only block comments: ⌘/ wraps, and accented text doesn't trip it.
        let xml = cx.new(|cx| Editor::new(Buffer::from_text("Été déjà\n"), Some(PathBuf::from("a.xml")), cx));
        xml.update(cx, |e, cx| {
            e.selection = Selection::caret(0);
            e.toggle_comment(cx);
            assert_eq!(e.buffer.to_string(), "<!-- Été déjà -->\n");
            e.toggle_comment(cx);
            assert_eq!(e.buffer.to_string(), "Été déjà\n");
        });
    }

    /// A Swift file folds by its blocks, and knows the blocks around a line (sticky scroll).
    #[gpui::test]
    fn languages_without_a_grammar_fold(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
        });
        let text = "struct A {\n    func f() {\n        g()\n        h()\n    }\n}\n";
        let e = cx.new(|cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("a.swift")), cx));
        e.update(cx, |e, cx| {
            assert_eq!(e.foldable(), [0..5, 1..4]);
            assert_eq!(e.blocks_around(2), [1..4, 0..5], "innermost first");
            e.toggle_fold(1, cx);
            assert!(e.is_folded(1));
        });
    }

    /// The editing helpers of code files, in one without a grammar: a doc comment carried on,
    /// a `}` back out, comments spell-checked.
    #[gpui::test]
    fn languages_without_a_grammar_get_the_editing_helpers(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let typed = |cx: &mut TestAppContext, text: &str, keys: &[&str]| {
            let text = text.to_string();
            let (e, cx) =
                cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(PathBuf::from("a.swift")), cx));
            e.update_in(cx, |e, window, cx| {
                window.focus(&gpui::Focusable::focus_handle(e, cx));
                e.selection = Selection::caret(e.buffer.len_chars());
            });
            cx.run_until_parked();
            for key in keys {
                match *key {
                    "enter" => cx.simulate_keystrokes("enter"),
                    text => cx.simulate_input(text),
                }
            }
            e.read_with(cx, |e, _| e.buffer.to_string())
        };
        assert_eq!(typed(cx, "/// Adds one.", &["enter", "More."]), "/// Adds one.\n/// More.");
        assert_eq!(typed(cx, "func f() {\n    g()\n    ", &["}"]), "func f() {\n    g()\n}");
    }
}
