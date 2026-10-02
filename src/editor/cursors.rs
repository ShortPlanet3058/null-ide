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

    /// Alt+click: adds a cursor there, or removes the one already there.
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

    pub(super) fn copy_selections(&self, cx: &mut Context<Self>) -> bool {
        let texts = self.selected_texts();
        if texts.is_empty() {
            return false;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(texts.join("\n")));
        true
    }

    /// Pastes at every cursor. When the clipboard holds one line per cursor, each
    /// cursor gets its own line, so copying from several cursors and pasting works.
    pub(super) fn paste_text(&mut self, text: String, cx: &mut Context<Self>) {
        let count = self.extra.len() + 1;
        let lines: Vec<&str> = text.split('\n').collect();
        let split = count > 1 && lines.len() == count;
        // Cursors are visited bottom up, so hand out the lines from the end.
        let mut next = count;
        self.for_each_cursor(cx, |this, cx| {
            next = next.saturating_sub(1);
            let piece = if split { lines[next] } else { text.as_str() };
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

    #[gpui::test]
    fn alt_click_toggles_cursors_and_they_merge(cx: &mut TestAppContext) {
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
    fn pasting_one_line_per_cursor(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "x\ny\n");
        e.update(cx, |e, cx| {
            e.selection = Selection::caret(0);
            e.add_cursor_vertically(true, cx);
            e.paste_text("1\n2".into(), cx);
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
}
