//! Line-level editing commands: comments, indentation, moving and duplicating lines,
//! and the brackets and quotes that close themselves.

use super::{EditKind, Editor, EditorEvent, Selection, TAB_SIZE};
use gpui::Context;
use std::ops::Range;

/// Pairs that close themselves when the opening one is typed.
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

    fn lines_char_range(&self, lines: &Range<usize>) -> Range<usize> {
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
    fn rewrite_lines(
        &mut self,
        lines: Range<usize>,
        new_lines: Vec<String>,
        place: impl Fn((usize, usize)) -> (usize, usize),
        cx: &mut Context<Self>,
    ) {
        let range = self.lines_char_range(&lines);
        let had_newline = self.buffer.slice(range.clone()).ends_with('\n');
        let mut text = new_lines.join("\n");
        if had_newline {
            text.push('\n');
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

    fn line_texts(&self, lines: &Range<usize>) -> Vec<String> {
        lines.clone().map(|l| self.buffer.line_text(l)).collect()
    }

    // ---------- comments ----------

    /// Comments the selected lines out, or back in when they all already are.
    pub(super) fn toggle_comment(&mut self, cx: &mut Context<Self>) {
        let Some(language) = self.language() else { return };
        let lines = self.selected_lines();
        let texts = self.line_texts(&lines);
        if let Some(marker) = language.line_comment {
            let (new_lines, shift) = toggle_line_comments(&texts, marker);
            let start = lines.start;
            self.rewrite_lines(lines, new_lines, |(l, c)| (l, shift(l - start, c)), cx);
        } else if let Some((open, close)) = language.block_comment {
            let new_lines = toggle_block_comment(&texts, open, close);
            self.rewrite_lines(lines, new_lines, |p| p, cx);
        }
    }

    // ---------- indentation ----------

    pub(super) fn indent_lines(&mut self, cx: &mut Context<Self>) {
        let lines = self.selected_lines();
        let unit = " ".repeat(TAB_SIZE);
        let new_lines = self
            .line_texts(&lines)
            .into_iter()
            .map(|t| if t.trim().is_empty() { t } else { format!("{unit}{t}") })
            .collect();
        self.rewrite_lines(lines, new_lines, |(l, c)| (l, c + TAB_SIZE), cx);
    }

    pub(super) fn outdent_lines(&mut self, cx: &mut Context<Self>) {
        let lines = self.selected_lines();
        let texts = self.line_texts(&lines);
        let removed: Vec<usize> = texts
            .iter()
            .map(|t| t.chars().take(TAB_SIZE).take_while(|c| *c == ' ').count().max(t.starts_with('\t') as usize))
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
        let total = self.buffer.len_lines();
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
        self.rewrite_lines(span, texts, |(l, c)| (if down { l + 1 } else { l - 1 }, c), cx);
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
        let Some(&(open, close)) = PAIRS.iter().find(|(o, cl)| *o == c || *cl == c) else { return false };
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
                let forward = c == open;
                let mut depth = 0i32;
                let mut j = i;
                for _ in 0..LIMIT {
                    match at(j) {
                        Some(x) if x == open => depth += if forward { 1 } else { -1 },
                        Some(x) if x == close => depth += if forward { -1 } else { 1 },
                        None => break,
                        _ => {}
                    }
                    if depth == 0 {
                        return Some((i, j));
                    }
                    if forward {
                        j += 1;
                    } else if j == 0 {
                        break;
                    } else {
                        j -= 1;
                    }
                }
            }
        }
        None
    }
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
                format!("{}{marker} {}", &line[..indent], &line[indent..])
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
    if first.starts_with(open) && last.ends_with(close) {
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
}
