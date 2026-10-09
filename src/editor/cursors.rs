//! Multiple cursors: every editing and movement command runs once per cursor,
//! as one undo step, and the text is re-highlighted once at the end.

use super::{CharClass, Editor, Selection, char_class};
use gpui::{ClipboardItem, Context};
use std::ops::Range;

/// A cursor besides the main one, with its own column to keep when moving up and down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub selection: Selection,
    pub goal: Option<usize>,
}

impl Cursor {
    fn new(selection: Selection) -> Self {
        Self { selection, goal: None }
    }
}

/// What's on the clipboard, as Null wrote it: whole lines, or one piece per cursor. After
/// a `|`, how indented the line it was copied from was, so pasting can move it to its new
/// depth.
const LINES: &str = "null-lines";
const PIECES: &str = "null-pieces";

/// Set while a command runs once per cursor.
pub(super) struct Batch {
    /// The cursors before the command, main one first, for undo.
    pub before: Vec<Selection>,
    pub recorded: bool,
    pub changed: bool,
    /// Whether the cursor being worked on is the main one.
    pub primary: bool,
}

impl Editor {
    /// True when there is more than one cursor (also while a command runs on each of them).
    pub fn multi_cursor(&self) -> bool {
        !self.extra.is_empty() || self.batch.is_some()
    }

    /// Every cursor's selection, main one first.
    pub(super) fn all_selections(&self) -> Vec<Selection> {
        std::iter::once(self.selection).chain(self.extra.iter().map(|c| c.selection)).collect()
    }

    /// Drops every cursor but the main one.
    pub fn single_cursor(&mut self) {
        self.extra.clear();
    }

    /// Runs `f` with each cursor in turn as the selection, from the bottom of the file up so
    /// an edit never moves the cursors still to do. Cursors already done are shifted by
    /// whatever the edit added or removed.
    pub(super) fn for_each_cursor(&mut self, cx: &mut Context<Self>, mut f: impl FnMut(&mut Self, &mut Context<Self>)) {
        if self.extra.is_empty() || self.batch.is_some() {
            return f(self, cx);
        }
        let mut cursors: Vec<(Cursor, bool)> =
            vec![(Cursor { selection: self.selection, goal: self.goal_column }, true)];
        cursors.extend(self.extra.drain(..).map(|c| (c, false)));
        let before = cursors.iter().map(|(c, _)| c.selection).collect();
        cursors.sort_by_key(|(c, _)| std::cmp::Reverse(c.selection.range().start));
        self.batch = Some(Batch { before, recorded: false, changed: false, primary: false });

        let mut done: Vec<(Cursor, bool)> = Vec::with_capacity(cursors.len());
        for (cursor, primary) in cursors {
            let len = self.buffer.len_chars();
            self.selection = cursor.selection;
            self.goal_column = cursor.goal;
            if let Some(batch) = &mut self.batch {
                batch.primary = primary;
            }
            f(self, cx);
            let delta = self.buffer.len_chars() as isize - len as isize;
            if delta != 0 {
                let floor = self.selection.anchor.max(self.selection.head) as isize;
                let max = self.buffer.len_chars() as isize;
                let shift = |x: &mut usize| *x = (*x as isize + delta).max(floor).min(max) as usize;
                for (c, _) in &mut done {
                    shift(&mut c.selection.anchor);
                    shift(&mut c.selection.head);
                }
            }
            done.push((Cursor { selection: self.selection, goal: self.goal_column }, primary));
        }

        let changed = self.batch.take().is_some_and(|b| b.changed);
        self.set_cursors(done);
        if changed {
            self.text_changed(cx);
        }
        cx.notify();
    }

    /// Whether the cursor `for_each_cursor` is working on is the main one.
    pub(super) fn on_primary_cursor(&self) -> bool {
        self.batch.as_ref().is_none_or(|b| b.primary)
    }

    /// Replaces all cursors, merging any that overlap. The flagged one becomes the main cursor.
    fn set_cursors(&mut self, mut cursors: Vec<(Cursor, bool)>) {
        cursors.sort_by_key(|(c, _)| (c.selection.range().start, c.selection.range().end));
        let mut merged: Vec<(Cursor, bool)> = Vec::with_capacity(cursors.len());
        for (cursor, primary) in cursors {
            let range = cursor.selection.range();
            if let Some((last, last_primary)) = merged.last_mut() {
                let last_range = last.selection.range();
                if range.start < last_range.end || range.start == last_range.start {
                    let union = last_range.start..range.end.max(last_range.end);
                    let reversed = cursor.selection.head < cursor.selection.anchor;
                    let selection = if reversed {
                        Selection { anchor: union.end, head: union.start }
                    } else {
                        Selection { anchor: union.start, head: union.end }
                    };
                    *last = Cursor { selection, goal: if primary { cursor.goal } else { last.goal } };
                    *last_primary |= primary;
                    continue;
                }
            }
            merged.push((cursor, primary));
        }
        let main = merged.iter().position(|(_, p)| *p).unwrap_or(merged.len().saturating_sub(1));
        let (cursor, _) = merged.remove(main);
        self.selection = cursor.selection;
        self.goal_column = cursor.goal;
        self.extra = merged.into_iter().map(|(c, _)| c).collect();
    }

    /// Before a line command: cursors that touch the same lines become one, so a
    /// line isn't commented or indented twice.
    pub(super) fn merge_cursors_on_shared_lines(&mut self) {
        if self.extra.is_empty() {
            return;
        }
        let main = self.selection;
        let mut groups: Vec<(Range<usize>, Selection, bool)> = Vec::new();
        let mut all = self.all_selections();
        all.sort_by_key(|s| s.range().start);
        for selection in all {
            self.selection = selection;
            let lines = self.selected_lines();
            let is_main = selection == main;
            match groups.last_mut() {
                Some((last_lines, last, last_main)) if lines.start < last_lines.end => {
                    let range = last.range().start..selection.range().end.max(last.range().end);
                    *last = Selection { anchor: range.start, head: range.end };
                    last_lines.end = last_lines.end.max(lines.end);
                    *last_main |= is_main;
                }
                _ => groups.push((lines, selection, is_main)),
            }
        }
        let cursors = groups.into_iter().map(|(_, s, m)| (Cursor::new(s), m)).collect();
        self.set_cursors(cursors);
    }

    // ---------- adding cursors ----------

    /// Before adding a cursor: how they were, for ⌘U to go back to.
    pub(super) fn remember_cursors(&mut self) {
        const KEPT: usize = 50;
        self.cursor_history.push((self.selection, self.extra.clone()));
        if self.cursor_history.len() > KEPT {
            self.cursor_history.remove(0);
        }
    }

    /// ⌘U: the cursors as they were before the last one was added.
    pub(super) fn restore_cursors(&mut self, cx: &mut Context<Self>) {
        let Some((selection, extra)) = self.cursor_history.pop() else { return };
        let len = self.buffer.len_chars();
        let fit = |s: Selection| Selection { anchor: s.anchor.min(len), head: s.head.min(len) };
        self.selection = fit(selection);
        self.extra = extra.into_iter().map(|c| Cursor { selection: fit(c.selection), goal: c.goal }).collect();
        self.goal_column = None;
        self.touch(cx);
    }

    /// ⌥⇧I: a cursor at the end of each line the selection covers.
    pub(super) fn cursors_at_line_ends(&mut self, cx: &mut Context<Self>) {
        let lines = self.selected_lines();
        if lines.len() < 2 {
            return;
        }
        let cursors = lines
            .clone()
            .map(|line| {
                let end = self.buffer.line_to_char(line) + self.buffer.line_len(line);
                (Cursor::new(Selection::caret(end)), line + 1 == lines.end)
            })
            .collect();
        self.set_cursors(cursors);
        self.touch(cx);
    }

    /// A box from `from` to `to` (line, column): the same columns selected on every line
    /// between, the cursor on `to`'s line the main one. Lines that end before the box
    /// starts are left out (unless it's no wider than a caret: then they get one at their end).
    pub(super) fn select_box(&mut self, from: (usize, usize), to: (usize, usize)) {
        let (first, last) = (from.0.min(to.0), from.0.max(to.0));
        let (left, right) = (from.1.min(to.1), from.1.max(to.1));
        let mut cursors = Vec::new();
        for line in first..=last.min(self.buffer.len_lines().saturating_sub(1)) {
            let len = self.buffer.line_len(line);
            if left >= len && left != right {
                continue;
            }
            let at = |col: usize| self.buffer.offset(line, col.min(len));
            let selection = Selection { anchor: at(from.1), head: at(to.1) };
            cursors.push((Cursor::new(selection), line == to.0));
        }
        if !cursors.iter().any(|(_, main)| *main)
            && let Some(last) = cursors.last_mut()
        {
            last.1 = true;
        }
        if !cursors.is_empty() {
            self.set_cursors(cursors);
        }
    }

    /// Cmd+Shift+click: adds a cursor there, or removes the one already there.
    pub(super) fn toggle_cursor_at(&mut self, offset: usize) {
        if !self.extra.is_empty() {
            if let Some(i) =
                self.extra.iter().position(|c| c.selection.range().contains(&offset) || c.selection.head == offset)
            {
                self.extra.remove(i);
                return;
            }
            if self.selection.head == offset {
                let next = self.extra.pop().expect("checked above");
                self.selection = next.selection;
                self.goal_column = next.goal;
                return;
            }
        }
        self.extra.push(Cursor { selection: self.selection, goal: self.goal_column });
        self.selection = Selection::caret(offset);
        self.goal_column = None;
        let cursors = self.take_cursors();
        self.set_cursors(cursors);
    }

    fn take_cursors(&mut self) -> Vec<(Cursor, bool)> {
        let mut cursors = vec![(Cursor { selection: self.selection, goal: self.goal_column }, true)];
        cursors.extend(self.extra.drain(..).map(|c| (c, false)));
        cursors
    }

    /// Adds a cursor on the line above the top cursor, or below the bottom one.
    pub(super) fn add_cursor_vertically(&mut self, down: bool, cx: &mut Context<Self>) {
        let mut cursors = self.take_cursors();
        let heads = cursors.iter().map(|(c, _)| c);
        let edge = if down { heads.max_by_key(|c| c.selection.head) } else { heads.min_by_key(|c| c.selection.head) };
        let edge = *edge.expect("there is always a main cursor");
        let (line, col) = self.buffer.point(edge.selection.head);
        let goal = edge.goal.unwrap_or(col);
        let target = if down { line + 1 } else { line.wrapping_sub(1) };
        if target < self.buffer.len_lines() {
            cursors.iter_mut().for_each(|(_, main)| *main = false);
            let offset = self.buffer.offset(target, goal);
            cursors.push((Cursor { selection: Selection::caret(offset), goal: Some(goal) }, true));
        }
        self.set_cursors(cursors);
        self.touch(cx);
    }

    /// The text to look for when adding occurrences, and whether only whole words count.
    /// With just a caret, this first selects the word under it and returns None: that
    /// press is done, the next one adds occurrences.
    fn occurrence_needle(&mut self) -> Option<(String, bool)> {
        if self.selection.is_empty() {
            let word = self.word_at(self.selection.head);
            if !word.is_empty() && self.buffer.char_at(word.start).map(char_class) == Some(CharClass::Word) {
                self.selection = Selection { anchor: word.start, head: word.end };
                self.word_pick = Some(word);
            }
            return None;
        }
        // A word picked by ⌘D only matches whole words; a hand-made selection matches anywhere.
        if self.extra.is_empty() {
            self.occurrence_whole_word = self.word_pick.as_ref() == Some(&self.selection.range());
        }
        Some((self.buffer.slice(self.selection.range()), self.occurrence_whole_word))
    }

    /// Every place `needle` appears, as char ranges.
    fn occurrences(&self, needle: &str, whole_word: bool) -> Vec<Range<usize>> {
        let text = self.buffer.to_string();
        let rope = self.buffer.rope();
        let is_word = |c: Option<char>| c.is_some_and(|c| char_class(c) == CharClass::Word);
        text.match_indices(needle)
            .map(|(byte, _)| {
                let start = rope.byte_to_char(byte);
                start..start + needle.chars().count()
            })
            .filter(|r| {
                !whole_word
                    || (!is_word(r.start.checked_sub(1).and_then(|i| self.buffer.char_at(i)))
                        && !is_word(self.buffer.char_at(r.end)))
            })
            .collect()
    }

    /// ⌘D: selects the word at the caret, then each press adds the next place it appears.
    pub(super) fn add_next_occurrence(&mut self, cx: &mut Context<Self>) {
        let Some((needle, whole_word)) = self.occurrence_needle() else {
            return self.touch(cx);
        };
        let taken: Vec<Range<usize>> = self.all_selections().iter().map(|s| s.range()).collect();
        let found = self.occurrences(&needle, whole_word);
        let after = self.selection.range().end;
        let next = found
            .iter()
            .filter(|r| r.start >= after)
            .chain(found.iter().filter(|r| r.start < after))
            .find(|r| !taken.contains(r))
            .cloned();
        if let Some(range) = next {
            let mut cursors: Vec<(Cursor, bool)> = self.take_cursors().into_iter().map(|(c, _)| (c, false)).collect();
            cursors.push((Cursor::new(Selection { anchor: range.start, head: range.end }), true));
            self.set_cursors(cursors);
        }
        self.touch(cx);
    }

    /// ⌃⌘D: the occurrence just picked (⌘D) let go, and the next one picked instead, so
    /// one that shouldn't change can be stepped over.
    pub(super) fn skip_occurrence(&mut self, cx: &mut Context<Self>) {
        let Some((needle, whole_word)) = self.occurrence_needle() else {
            return self.touch(cx);
        };
        let skipped = self.selection.range();
        let others: Vec<Range<usize>> = self.extra.iter().map(|c| c.selection.range()).collect();
        let found = self.occurrences(&needle, whole_word);
        let next = found
            .iter()
            .filter(|r| r.start >= skipped.end)
            .chain(found.iter().filter(|r| r.start < skipped.start))
            .find(|r| !others.contains(r))
            .cloned();
        if let Some(range) = next {
            // A word picked by ⌘D stays matched as a whole word from the one picked now.
            if whole_word {
                self.word_pick = Some(range.clone());
            }
            let mut cursors: Vec<(Cursor, bool)> = self.take_cursors().into_iter().filter(|(_, main)| !main).collect();
            cursors.push((Cursor::new(Selection { anchor: range.start, head: range.end }), true));
            self.set_cursors(cursors);
        }
        self.touch(cx);
    }

    /// ⌥↵ in the find bar: a cursor on every match (the current one the main cursor), the
    /// bar closed and the keyboard back in the text.
    pub fn select_all_matches(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.search_now();
        let Some(search) = &self.search else { return };
        let current = search.current;
        let cursors: Vec<(Cursor, bool)> = search
            .matches
            .iter()
            .enumerate()
            .map(|(i, r)| (Cursor::new(Selection { anchor: r.start, head: r.end }), Some(i) == current))
            .collect();
        if cursors.is_empty() {
            return;
        }
        self.close_find(window, cx);
        self.set_cursors(cursors);
        self.touch(cx);
    }

    /// ⌘⇧L: a cursor on every place the selection (or the word at the caret) appears.
    pub(super) fn select_all_occurrences(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.occurrence_needle();
        }
        let Some((needle, whole_word)) = self.occurrence_needle() else { return self.touch(cx) };
        let main = self.selection.range();
        let cursors = self
            .occurrences(&needle, whole_word)
            .into_iter()
            .map(|r| (Cursor::new(Selection { anchor: r.start, head: r.end }), r == main))
            .collect::<Vec<_>>();
        if !cursors.is_empty() {
            self.set_cursors(cursors);
        }
        self.touch(cx);
    }

    // ---------- clipboard ----------

    /// The selected texts of every cursor, top to bottom.
    fn selected_texts(&self) -> Vec<String> {
        let mut ranges: Vec<Range<usize>> = self.all_selections().iter().map(|s| s.range()).collect();
        ranges.sort_by_key(|r| r.start);
        ranges.into_iter().filter(|r| !r.is_empty()).map(|r| self.buffer.slice(r)).collect()
    }

    /// Copies the selections; with nothing selected, the whole lines the cursors are on,
    /// marked as such so pasting puts them back as lines.
    pub(super) fn copy_selections(&self, cx: &mut Context<Self>) -> bool {
        let texts = self.selected_texts();
        if texts.is_empty() {
            let mut lines: Vec<usize> = self.all_selections().iter().map(|s| self.buffer.point(s.head).0).collect();
            lines.sort();
            lines.dedup();
            let text: String = lines.iter().map(|&l| format!("{}\n", self.buffer.line_text(l))).collect();
            let indent = self.line_indent(lines[0]);
            let item = ClipboardItem::new_string_with_metadata(text, format!("{LINES}|{indent}"));
            crate::clipboard_history::remember(&item, cx);
            cx.write_to_clipboard(item);
            return true;
        }
        let kind = if texts.len() > 1 {
            PIECES.to_string()
        } else {
            format!("|{}", self.line_indent(self.buffer.point(self.selection.range().start).0))
        };
        let item = ClipboardItem::new_string_with_metadata(texts.join("\n"), kind);
        crate::clipboard_history::remember(&item, cx);
        cx.write_to_clipboard(item);
        true
    }

    /// How wide line `line`'s indentation shows.
    fn line_indent(&self, line: usize) -> usize {
        super::reindent::indent_columns(&self.buffer.line_text(line), super::TAB_SIZE)
    }

    /// How indented lines put in at the start of `line` should be: as deep as that line;
    /// a closing `}` takes them as deep as the block it ends; a blank line as deep as its own
    /// spaces, or inside the block the line above opens.
    fn indent_for_lines_at(&self, line: usize) -> usize {
        let tab = super::TAB_SIZE;
        let above = (line.saturating_sub(1000)..line).rev().find(|&l| !self.buffer.line_text(l).trim().is_empty());
        let after_above = |l: usize| super::reindent::indent_columns(&self.indent_after(l), tab);
        let own = self.line_indent(line);
        let text = self.buffer.line_text(line);
        let words = text.trim_start();
        if words.is_empty() {
            let opened = above.filter(|&l| after_above(l) > self.line_indent(l));
            return if own > 0 { own } else { opened.map_or(0, after_above) };
        }
        if words.starts_with(['}', ')', ']']) {
            return own.max(above.map_or(0, after_above));
        }
        own
    }

    /// `text` moved to the indentation of where it goes in at `range`, and where it goes in
    /// (from the start of the line, when it's put in within the indentation).
    fn reindented(&self, text: &str, base: Option<usize>, range: Range<usize>) -> (String, Range<usize>) {
        let tab = super::TAB_SIZE;
        let base = base.unwrap_or_else(|| super::reindent::indent_columns(text, tab));
        let unit = self.style.indent;
        let (line, column) = self.buffer.point(range.start);
        let line_text = self.buffer.line_text(line);
        let before: String = line_text.chars().take(column).collect();
        if !before.trim().is_empty() {
            let target = self.line_indent(line);
            return (super::reindent::reindent(text, base, target, false, unit, tab), range);
        }
        let blank = line_text.trim().is_empty();
        let whole_lines = column == 0 && text.ends_with('\n');
        let target = if blank || whole_lines {
            self.indent_for_lines_at(line).max(super::reindent::indent_columns(&before, tab))
        } else {
            super::reindent::indent_columns(&before, tab)
        };
        let start = self.buffer.line_to_char(line);
        (super::reindent::reindent(text, base, target, true, unit, tab), start..range.end)
    }

    /// Pastes at every cursor. Whole lines (copied with nothing selected) go in above
    /// the cursor's line; text copied from several cursors goes one piece per cursor.
    /// With `adjust`, code of several lines moves to the indentation where it goes.
    pub(super) fn paste_text(&mut self, text: String, meta: &str, adjust: bool, cx: &mut Context<Self>) {
        let (kind, base) = meta.split_once('|').map_or((meta, None), |(kind, indent)| (kind, indent.parse().ok()));
        let adjust =
            adjust && text.contains('\n') && kind != PIECES && self.language().is_some() && !self.is_markdown();
        if kind == LINES && self.all_selections().iter().all(|s| s.is_empty()) {
            return self.for_each_cursor(cx, |this, cx| {
                let caret = this.selection.head;
                let start = this.buffer.line_to_char(this.buffer.point(caret).0);
                let text = if adjust { this.reindented(&text, base, start..start).0 } else { text.clone() };
                this.edit(start..start, &text, super::EditKind::Other, cx);
                this.selection = Selection::caret(caret + text.chars().count());
            });
        }
        if adjust {
            return self.for_each_cursor(cx, |this, cx| {
                let (text, range) = this.reindented(&text, base, this.selection.range());
                this.edit(range, &text, super::EditKind::Other, cx);
            });
        }
        let count = self.extra.len() + 1;
        let lines: Vec<&str> = text.split('\n').collect();
        let split = kind == PIECES && count > 1 && lines.len() == count;
        // Cursors are visited bottom up, so hand out the lines from the end.
        let mut next = count;
        self.for_each_cursor(cx, |this, cx| {
            next = next.saturating_sub(1);
            // A "\r\n" file's pieces: without the "\r" that came before each "\n".
            let piece = if split { lines[next].trim_end_matches('\r') } else { text.as_str() };
            this.edit(this.selection.range(), piece, super::EditKind::Other, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::buffer::Buffer;
    use crate::editor::{Editor, Selection};
    use gpui::{EntityInputHandler as _, TestAppContext, VisualTestContext};
    use std::path::PathBuf;

    fn editor<'a>(cx: &'a mut TestAppContext, text: &str) -> (gpui::Entity<Editor>, &'a mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = text.to_string();
        cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(PathBuf::from("x.py")), cx))
    }

    fn text(cx: &mut VisualTestContext, e: &gpui::Entity<Editor>) -> String {
        e.read_with(cx, |e, _| e.buffer.to_string())
    }

    fn type_text(cx: &mut VisualTestContext, e: &gpui::Entity<Editor>, text: &str) {
        e.update_in(cx, |e, window, cx| e.replace_text_in_range(None, text, window, cx));
    }

    fn cursor_count(cx: &mut VisualTestContext, e: &gpui::Entity<Editor>) -> usize {
        e.read_with(cx, |e, _| e.extra.len() + 1)
    }

    #[gpui::test]
    fn cmd_d_picks_the_word_then_adds_whole_word_occurrences(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "foo = foobar + foo\nfoo()\n");
        e.update(cx, |e, cx| {
            e.selection = Selection::caret(1);
            e.add_next_occurrence(cx);
        });
        assert_eq!(cursor_count(cx, &e), 1);
        assert_eq!(e.read_with(cx, |e, _| e.selection.range()), 0..3);
        e.update(cx, |e, cx| e.add_next_occurrence(cx));
        e.update(cx, |e, cx| e.add_next_occurrence(cx));
        // "foobar" is skipped: only the whole word counts.
        assert_eq!(cursor_count(cx, &e), 3);
        type_text(cx, &e, "bar");
        assert_eq!(text(cx, &e), "bar = foobar + bar\nbar()\n");
        // Typing with several cursors is one undo step.
        e.update(cx, |e, cx| e.step_history(true, cx));
        assert_eq!(text(cx, &e), "foo = foobar + foo\nfoo()\n");
        assert_eq!(cursor_count(cx, &e), 3);
    }

    /// ⌃⌘D steps over the one just picked: it's let go, the next one picked.
    #[gpui::test]
    fn an_occurrence_can_be_skipped(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "x = 1\nx = 2\nx = 3\nx = 4\n");
        e.update(cx, |e, cx| {
            e.selection = Selection::caret(0);
            e.add_next_occurrence(cx);
            e.add_next_occurrence(cx);
            // The second `x` shouldn't change: the third instead.
            e.skip_occurrence(cx);
            assert_eq!(e.selection.range(), 12..13);
            e.add_next_occurrence(cx);
        });
        assert_eq!(cursor_count(cx, &e), 3);
        type_text(cx, &e, "y");
        assert_eq!(text(cx, &e), "y = 1\nx = 2\ny = 3\ny = 4\n");
    }

    /// Alone, skipping moves the one selection on.
    #[gpui::test]
    fn skipping_the_only_one_moves_it(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "a b a\n");
        e.update(cx, |e, cx| {
            e.selection = Selection { anchor: 0, head: 1 };
            e.skip_occurrence(cx);
            assert_eq!((e.selection.range(), e.extra.len()), (4..5, 0));
        });
        // A word picked with ⌘D, skipped alone: still whole words after (not the `a` in `ab`).
        let (e, cx) = editor(cx, "a b a ab a\n");
        e.update(cx, |e, cx| {
            e.selection = Selection::caret(0);
            e.add_next_occurrence(cx);
            e.skip_occurrence(cx);
            assert_eq!(e.selection.range(), 4..5);
            e.add_next_occurrence(cx);
            assert_eq!(e.selection.range(), 9..10);
        });
    }

    #[gpui::test]
    fn select_all_occurrences_and_delete(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "a.b a.b a.c\n");
        e.update(cx, |e, cx| {
            e.selection = Selection { anchor: 0, head: 3 };
            e.select_all_occurrences(cx);
        });
        assert_eq!(cursor_count(cx, &e), 2);
        e.update(cx, |e, cx| {
            e.for_each_cursor(cx, |this, cx| {
                this.edit(this.selection.range(), "", super::super::EditKind::Deleting, cx)
            })
        });
        assert_eq!(text(cx, &e), "  a.c\n");
    }

    #[gpui::test]
    fn cursors_above_and_below_type_together(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "one\ntwo\nthree\n");
        e.update(cx, |e, cx| {
            e.selection = Selection::caret(1);
            e.add_cursor_vertically(true, cx);
            e.add_cursor_vertically(true, cx);
        });
        assert_eq!(cursor_count(cx, &e), 3);
        type_text(cx, &e, "-");
        assert_eq!(text(cx, &e), "o-ne\nt-wo\nt-hree\n");
        // Brackets close themselves at every cursor.
        type_text(cx, &e, "(");
        assert_eq!(text(cx, &e), "o-(ne\nt-(wo\nt-(hree\n");
    }

    /// ⌥⇧ and a drag, with the real mouse events: a box from where it went down to where
    /// it's let go.
    #[gpui::test]
    fn alt_shift_dragging_selects_a_box(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "abcd\nabcdef\nabcd\n");
        let draw = |cx: &mut VisualTestContext| {
            let e = e.clone();
            cx.draw(gpui::Point::default(), gpui::size(gpui::px(800.), gpui::px(400.)), move |_, _| {
                gpui::AnyView::from(e)
            });
        };
        draw(cx);
        // Where column `col` of line `line` is on screen, from the last frame's layout.
        let at = |cx: &mut VisualTestContext, line: usize, col: usize| {
            e.read_with(cx, |e, _| {
                let l = e.layout.as_ref().unwrap();
                gpui::point(
                    l.text_origin.x + l.char_width * (col as f32 + 0.1),
                    l.text_origin.y + l.line_height * (line as f32 + 0.5),
                )
            })
        };
        let box_keys = gpui::Modifiers { alt: true, shift: true, ..Default::default() };
        let (from, to) = (at(cx, 0, 1), at(cx, 2, 3));
        cx.simulate_mouse_down(from, gpui::MouseButton::Left, box_keys);
        for step in 1..=4 {
            let t = step as f32 / 4.;
            let p = gpui::point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            cx.simulate_mouse_move(p, gpui::MouseButton::Left, box_keys);
            draw(cx);
        }
        cx.simulate_mouse_up(to, gpui::MouseButton::Left, box_keys);
        assert_eq!(cursor_count(cx, &e), 3);
        type_text(cx, &e, "X");
        assert_eq!(text(cx, &e), "aXd\naXdef\naXd\n");
    }

    /// A click on a letter's left half puts the caret before it, its right half after it,
    /// the line's last letter too.
    #[gpui::test]
    fn a_click_lands_on_the_nearer_side_of_a_letter(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "abcd\n");
        let e2 = e.clone();
        cx.draw(gpui::Point::default(), gpui::size(gpui::px(800.), gpui::px(400.)), move |_, _| {
            gpui::AnyView::from(e2)
        });
        let caret_after_click = |cx: &mut VisualTestContext, col: f32| {
            let at = e.read_with(cx, |e, _| {
                let l = e.layout.as_ref().unwrap();
                gpui::point(l.text_origin.x + l.char_width * col, l.text_origin.y + l.line_height * 0.5)
            });
            cx.simulate_click(at, gpui::Modifiers::default());
            e.read_with(cx, |e, _| e.caret_point())
        };
        assert_eq!(caret_after_click(cx, 1.3), (0, 1));
        assert_eq!(caret_after_click(cx, 1.7), (0, 2));
        // The last letter, "d": its left half is before it.
        assert_eq!(caret_after_click(cx, 3.3), (0, 3));
        assert_eq!(caret_after_click(cx, 3.7), (0, 4));
    }

    #[gpui::test]
    fn a_box_selects_the_same_columns_on_each_line(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "abcd\nabcdef\na\nabcd\n");
        // From column 1 of the first line to column 3 of the fourth: "bc" on each line
        // long enough; the one-letter line is left out.
        e.update(cx, |e, _| e.select_box((0, 1), (3, 3)));
        assert_eq!(cursor_count(cx, &e), 3);
        type_text(cx, &e, "X");
        assert_eq!(text(cx, &e), "aXd\naXdef\na\naXd\n");
        // No wider than a caret: every line gets one, short ones at their end.
        e.update(cx, |e, _| e.select_box((0, 2), (2, 2)));
        assert_eq!(cursor_count(cx, &e), 3);
        type_text(cx, &e, "|");
        assert_eq!(text(cx, &e), "aX|d\naX|def\na|\naXd\n");
    }

    #[gpui::test]
    fn cursors_at_line_ends_and_undo_cursor(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "let a = 1\nlet b = 2\nlet c = 3\n");
        e.update(cx, |e, cx| {
            e.selection = Selection { anchor: 0, head: 25 };
            e.remember_cursors();
            e.cursors_at_line_ends(cx);
        });
        assert_eq!(cursor_count(cx, &e), 3);
        type_text(cx, &e, ";");
        assert_eq!(text(cx, &e), "let a = 1;\nlet b = 2;\nlet c = 3;\n");
        // ⌘D on `let` three times (the word, then two more), then ⌘U takes the last back.
        e.update(cx, |e, cx| {
            e.selection = Selection::caret(0);
            e.single_cursor();
            e.add_next_occurrence(cx);
            e.remember_cursors();
            e.add_next_occurrence(cx);
            e.remember_cursors();
            e.add_next_occurrence(cx);
        });
        assert_eq!(cursor_count(cx, &e), 3);
        e.update(cx, |e, cx| e.restore_cursors(cx));
        assert_eq!(cursor_count(cx, &e), 2);
    }

    #[gpui::test]
    fn cmd_shift_click_toggles_cursors_and_they_merge(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "abc\nabc\n");
        e.update(cx, |e, _| {
            e.selection = Selection::caret(0);
            e.toggle_cursor_at(4);
        });
        assert_eq!(cursor_count(cx, &e), 2);
        e.update(cx, |e, _| e.toggle_cursor_at(0));
        assert_eq!(cursor_count(cx, &e), 1);
        e.update(cx, |e, _| e.toggle_cursor_at(1));
        // Backspacing both into the same spot leaves one cursor.
        e.update(cx, |e, cx| {
            e.selection = Selection::caret(1);
            e.extra = vec![super::Cursor { selection: Selection::caret(2), goal: None }];
            e.for_each_cursor(cx, |this, cx| {
                let head = this.selection.head;
                this.edit(head - 1..head, "", super::super::EditKind::Deleting, cx)
            });
        });
        assert_eq!(text(cx, &e), "c\nabc\n");
        assert_eq!(cursor_count(cx, &e), 1);
    }

    #[gpui::test]
    fn enter_inside_the_indentation_does_not_double_it(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "    foo\n");
        e.update_in(cx, |e, window, cx| {
            e.selection = Selection::caret(0);
            e.newline(&crate::editor::Newline, window, cx);
        });
        assert_eq!(text(cx, &e), "\n    foo\n");
        // On a line of only spaces, the spaces don't stay behind.
        e.update_in(cx, |e, window, cx| {
            e.buffer.replace(0..e.buffer.len_chars(), "    \n");
            e.selection = Selection::caret(4);
            e.newline(&crate::editor::Newline, window, cx);
        });
        assert_eq!(text(cx, &e), "\n    \n");
    }

    #[gpui::test]
    fn moving_the_last_line_down_adds_nothing(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "a\nb\n");
        e.update(cx, |e, cx| {
            e.selection = Selection::caret(2);
            e.move_lines(true, cx);
            e.move_lines(true, cx);
        });
        assert_eq!(text(cx, &e), "a\nb\n");
    }

    #[gpui::test]
    fn tab_twice_indents_the_same_lines(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "a\nb\n");
        e.update(cx, |e, cx| {
            e.selection = Selection::caret(0);
            e.select_line(cx);
            e.tab_key(cx);
            e.tab_key(cx);
        });
        assert_eq!(text(cx, &e), "        a\nb\n");
    }

    #[gpui::test]
    fn undoing_back_to_the_saved_text_counts_as_saved(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "x\n");
        e.update(cx, |e, cx| {
            e.edit(0..0, "y", crate::editor::EditKind::Other, cx);
            assert!(e.buffer.is_dirty());
            e.step_history(true, cx);
            assert!(!e.buffer.is_dirty());
        });
    }

    #[gpui::test]
    fn arrows_step_over_whole_emoji(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "a👍🏽b\n");
        e.update(cx, |e, _| {
            // The thumbs-up and its skin tone are two code points, one character on screen.
            assert_eq!(e.right_of(1), 3);
            assert_eq!(e.left_of(3), 1);
        });
    }

    #[gpui::test]
    fn copy_with_nothing_selected_copies_the_line_and_pastes_it_above(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "one\ntwo\n");
        e.update_in(cx, |e, window, cx| {
            e.selection = Selection::caret(5);
            e.copy(&crate::editor::Copy, window, cx);
            e.paste(&crate::editor::Paste, window, cx);
        });
        assert_eq!(text(cx, &e), "one\ntwo\ntwo\n");
        // The caret stays on the same text, now one line down.
        assert_eq!(e.read_with(cx, |e, _| e.caret_point()), (2, 1));
    }

    #[gpui::test]
    fn pasting_one_line_per_cursor(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "x\ny\n");
        e.update(cx, |e, cx| {
            e.selection = Selection::caret(0);
            e.add_cursor_vertically(true, cx);
            e.paste_text("1\n2".into(), super::PIECES, true, cx);
        });
        assert_eq!(text(cx, &e), "1x\n2y\n");
    }

    #[gpui::test]
    fn line_commands_run_once_per_line(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "a = 1\nb = 2\n");
        e.update(cx, |e, cx| {
            // Two cursors on the first line, one on the second.
            e.selection = Selection::caret(0);
            e.extra = vec![
                super::Cursor { selection: Selection::caret(3), goal: None },
                super::Cursor { selection: Selection::caret(7), goal: None },
            ];
            e.merge_cursors_on_shared_lines();
            e.for_each_cursor(cx, |this, cx| this.toggle_comment(cx));
        });
        assert_eq!(text(cx, &e), "# a = 1\n# b = 2\n");
    }

    #[gpui::test]
    fn pieces_pasted_in_a_crlf_file_keep_no_stray_return(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "a\r\nb\r\n");
        e.update_in(cx, |e, _, cx| {
            e.set_cursors(vec![
                (super::Cursor::new(Selection::caret(0)), true),
                (super::Cursor::new(Selection::caret(3)), false),
            ]);
            e.paste_text("1\r\n2".into(), super::PIECES, true, cx);
            assert_eq!(e.buffer.to_string(), "1a\r\n2b\r\n");
        });
    }
}
