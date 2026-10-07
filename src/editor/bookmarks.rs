//! Bookmarks: lines to come back to. ⌘F2 marks the caret's line (its number turns the
//! accent colour), F3 and ⇧F3 go from one to the next, and **Bookmarks…** lists them in every
//! open file. They move with the code as it's edited, and are kept with the session.

use super::{Editor, EditorEvent};
use gpui::{App, Context, KeyBinding, Window, actions};

actions!(bookmarks, [ToggleBookmark, NextBookmark, PreviousBookmark]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-f2", ToggleBookmark, Some("Editor")),
        KeyBinding::new("f3", NextBookmark, Some("Editor")),
        KeyBinding::new("shift-f3", PreviousBookmark, Some("Editor")),
    ]);
}

/// The bookmark after `line` (or before it, going back), round to the other end.
fn next_after(bookmarks: &[usize], line: usize, forward: bool) -> Option<usize> {
    if forward {
        bookmarks.iter().copied().find(|&l| l > line).or(bookmarks.first().copied())
    } else {
        bookmarks.iter().rev().copied().find(|&l| l < line).or(bookmarks.last().copied())
    }
}

impl Editor {
    pub(super) fn toggle_bookmark(&mut self, _: &ToggleBookmark, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.buffer.point(self.selection.head).0;
        match self.bookmarks.binary_search(&line) {
            Ok(i) => {
                self.bookmarks.remove(i);
            }
            Err(i) => self.bookmarks.insert(i, line),
        }
        self.bookmarks_revision = self.buffer.revision();
        cx.emit(EditorEvent::BookmarksChanged);
        cx.notify();
    }

    pub(super) fn next_bookmark(&mut self, _: &NextBookmark, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_bookmark(true, cx);
    }

    pub(super) fn previous_bookmark(&mut self, _: &PreviousBookmark, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_bookmark(false, cx);
    }

    fn go_to_bookmark(&mut self, forward: bool, cx: &mut Context<Self>) {
        let line = self.buffer.point(self.selection.head).0;
        if let Some(target) = next_after(&self.bookmarks, line, forward) {
            self.go_to_line(target + 1, cx);
        }
    }

    pub fn is_bookmarked(&self, line: usize) -> bool {
        self.bookmarks.binary_search(&line).is_ok()
    }

    /// After an edit: the bookmarks move with their lines.
    pub(super) fn bookmarks_after_edit(&mut self, cx: &mut Context<Self>) {
        let revision = self.buffer.revision();
        let edits = (!self.bookmarks.is_empty()).then(|| self.buffer.edits_since(self.bookmarks_revision)).flatten();
        self.bookmarks_revision = revision;
        let Some(edits) = edits else { return };
        let before = self.bookmarks.clone();
        for edit in edits {
            let (start, old_end, new_end) = (edit.start.0, edit.old_end.0, edit.new_end.0);
            for line in &mut self.bookmarks {
                *line = super::breakpoints::move_line(*line, start, old_end, new_end);
            }
        }
        self.bookmarks.dedup();
        if self.bookmarks != before {
            cx.emit(EditorEvent::BookmarksChanged);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use gpui::TestAppContext;
    use std::path::PathBuf;

    #[test]
    fn going_round_the_bookmarks() {
        let marks = [2, 5, 9];
        assert_eq!(next_after(&marks, 0, true), Some(2));
        assert_eq!(next_after(&marks, 5, true), Some(9));
        assert_eq!(next_after(&marks, 9, true), Some(2), "from the last, back to the first");
        assert_eq!(next_after(&marks, 5, false), Some(2));
        assert_eq!(next_after(&marks, 1, false), Some(9), "from before the first, round to the last");
        assert_eq!(next_after(&[], 3, true), None);
    }

    /// By keystroke: ⌘F2 marks lines, F3 and ⇧F3 go round them, and they follow edits.
    #[gpui::test]
    fn bookmarks_mark_lines_and_follow_the_text(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            bind_keys(cx);
        });
        let text = "zero\none\ntwo\nthree\nfour\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("x.txt")), cx));
        e.update_in(cx, |e, window, _| window.focus(&e.focus_handle));
        let caret_line = |cx: &mut gpui::VisualTestContext| e.read_with(cx, |e, _| e.caret_point().0);
        let go = |cx: &mut gpui::VisualTestContext, line: usize| e.update(cx, |e, cx| e.go_to_line(line + 1, cx));
        go(cx, 1);
        cx.simulate_keystrokes("cmd-f2");
        go(cx, 3);
        cx.simulate_keystrokes("cmd-f2");
        assert_eq!(e.read_with(cx, |e, _| e.bookmarks.clone()), vec![1, 3]);
        cx.simulate_keystrokes("f3");
        assert_eq!(caret_line(cx), 1, "round from the last to the first");
        cx.simulate_keystrokes("f3");
        assert_eq!(caret_line(cx), 3);
        cx.simulate_keystrokes("shift-f3");
        assert_eq!(caret_line(cx), 1);
        // A line added above moves both down; ⌘F2 again takes one away.
        e.update(cx, |e, cx| {
            e.selection = super::super::Selection::caret(0);
            e.edit(0..0, "new\n", super::super::EditKind::Other, cx);
        });
        assert_eq!(e.read_with(cx, |e, _| e.bookmarks.clone()), vec![2, 4]);
        go(cx, 4);
        cx.simulate_keystrokes("cmd-f2");
        assert_eq!(e.read_with(cx, |e, _| e.bookmarks.clone()), vec![2]);
        assert!(e.read_with(cx, |e, _| e.is_bookmarked(2)));
    }
}
