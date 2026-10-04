//! Reviewing what an AI task changed in a file: each change shows as an inline diff, as
//! ⌘I's do, the old lines struck through above the new ones. ⇥ keeps the change at the
//! caret, Esc undoes it, and the caret moves on to the next.

use super::{EditKind, Editor, EditorEvent, Selection};
use gpui::{App, Context, KeyBinding, Window, actions};
use std::ops::Range;

actions!(review, [KeepHunk, UndoHunk]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Editor && review");
    cx.bind_keys([KeyBinding::new("tab", KeepHunk, ctx), KeyBinding::new("escape", UndoHunk, ctx)]);
}

/// One change: lines of the text before (`old`) that became lines of the text now (`new`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
    /// The old lines, without their line breaks, to show struck through.
    pub old_lines: Vec<String>,
}

pub(super) struct Review {
    /// The text as it was before the task, with the changes kept so far applied.
    base: String,
    pub hunks: Vec<Hunk>,
    /// The buffer revision `hunks` was worked out for.
    revision: u64,
}

/// The changes between two texts, by lines.
pub fn hunks(before: &str, after: &str) -> Vec<Hunk> {
    let old: Vec<&str> = before.split_inclusive('\n').collect();
    similar::TextDiff::from_lines(before, after)
        .ops()
        .iter()
        .filter_map(|op| {
            let (old_range, new_range) = match *op {
                similar::DiffOp::Equal { .. } => return None,
                similar::DiffOp::Delete { old_index, old_len, new_index } => {
                    (old_index..old_index + old_len, new_index..new_index)
                }
                similar::DiffOp::Insert { old_index, new_index, new_len } => {
                    (old_index..old_index, new_index..new_index + new_len)
                }
                similar::DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                    (old_index..old_index + old_len, new_index..new_index + new_len)
                }
            };
            let old_lines =
                old[old_range.clone()].iter().map(|l| l.trim_end_matches(['\n', '\r']).to_string()).collect();
            Some(Hunk { old: old_range, new: new_range, old_lines })
        })
        .collect()
}

/// Byte range of lines `lines` in `text`, line breaks included.
fn line_bytes(text: &str, lines: &Range<usize>) -> Range<usize> {
    let mut start = text.len();
    let mut end = text.len();
    let mut offset = 0;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        if i == lines.start {
            start = offset;
        }
        if i == lines.end {
            end = offset;
            break;
        }
        offset += line.len();
    }
    start.min(end)..end
}

impl Editor {
    /// Shows what changed since `before` as changes to keep or undo.
    pub fn start_review(&mut self, before: String, cx: &mut Context<Self>) {
        self.review = Some(Review { base: before, hunks: Vec::new(), revision: u64::MAX });
        self.refresh_review();
        if self.review.as_ref().is_some_and(|r| r.hunks.is_empty()) {
            self.review = None;
        }
        self.go_to_hunk(0, cx);
        self.rebuild_blocks();
        cx.notify();
    }

    pub fn in_review(&self) -> bool {
        self.review.is_some()
    }

    /// Stops showing the changes (they stay as they are).
    pub fn end_review(&mut self, cx: &mut Context<Self>) {
        if self.review.take().is_some() {
            self.rebuild_blocks();
            cx.notify();
        }
    }

    /// Works the changes out again after an edit.
    pub(super) fn refresh_review(&mut self) {
        let revision = self.buffer.revision();
        let Some(review) = &mut self.review else { return };
        if review.revision != revision {
            review.hunks = hunks(&review.base, &self.buffer.to_string());
            review.revision = revision;
        }
    }

    /// The change the keys act on: the one the caret is in, or the next one below it.
    pub(super) fn current_hunk(&self) -> Option<usize> {
        let review = self.review.as_ref()?;
        let line = self.buffer.point(self.selection.head).0;
        review
            .hunks
            .iter()
            .position(|h| line < h.new.end.max(h.new.start + 1))
            .or_else(|| review.hunks.len().checked_sub(1))
    }

    /// The lines changes added, tinted like ⌘I's.
    pub(super) fn review_added_lines(&self) -> Vec<Range<usize>> {
        self.review.iter().flat_map(|r| r.hunks.iter().map(|h| h.new.clone())).filter(|r| !r.is_empty()).collect()
    }

    fn go_to_hunk(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(hunk) = self.review.as_ref().and_then(|r| r.hunks.get(index.min(r.hunks.len().saturating_sub(1))))
        else {
            return;
        };
        let line = hunk.new.start;
        self.single_cursor();
        self.selection = Selection::caret(self.buffer.line_to_char(line));
        self.goal_column = None;
        self.touch(cx);
    }

    /// ⇥: the change stays as it is.
    pub(super) fn keep_hunk(&mut self, _: &KeepHunk, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_review();
        let Some(index) = self.current_hunk() else { return };
        let text = self.buffer.to_string();
        let Some(review) = &mut self.review else { return };
        let hunk = review.hunks[index].clone();
        // The base takes the new lines, so this is no longer a change.
        let old = line_bytes(&review.base, &hunk.old);
        let new = line_bytes(&text, &hunk.new);
        review.base.replace_range(old, &text[new]);
        review.revision = u64::MAX;
        self.after_hunk(index, cx);
    }

    /// Esc: the change is undone, the old lines put back.
    pub(super) fn undo_hunk(&mut self, _: &UndoHunk, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_review();
        let Some(index) = self.current_hunk() else { return };
        let Some(review) = &self.review else { return };
        let hunk = review.hunks[index].clone();
        let old_text = review.base[line_bytes(&review.base, &hunk.old)].to_string();
        let start = self.buffer.line_to_char(hunk.new.start);
        let end = if hunk.new.end >= self.buffer.len_lines() {
            self.buffer.len_chars()
        } else {
            self.buffer.line_to_char(hunk.new.end)
        };
        self.record_undo(EditKind::Other);
        self.buffer.replace(start..end, &old_text);
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.after_hunk(index, cx);
    }

    /// On to the next change, or the end of the review.
    fn after_hunk(&mut self, index: usize, cx: &mut Context<Self>) {
        self.refresh_review();
        if self.review.as_ref().is_some_and(|r| r.hunks.is_empty()) {
            self.review = None;
            cx.emit(EditorEvent::Reviewed);
        } else {
            self.go_to_hunk(index, cx);
        }
        self.rebuild_blocks();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_are_found_by_lines() {
        let found = hunks("a\nb\nc\nd\n", "a\nB\nc\nd\ne\n");
        assert_eq!(
            found,
            vec![
                Hunk { old: 1..2, new: 1..2, old_lines: vec!["b".into()] },
                Hunk { old: 4..4, new: 4..5, old_lines: vec![] },
            ]
        );
        assert_eq!(line_bytes("a\nb\nc", &(1..2)), 2..4);
        assert_eq!(line_bytes("a\nb\nc", &(2..3)), 4..5);
        assert_eq!(line_bytes("a\nb\n", &(2..2)), 4..4);
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    use crate::buffer::Buffer;
    use gpui::TestAppContext;
    use std::path::PathBuf;

    #[gpui::test]
    fn changes_are_kept_or_undone_one_by_one(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let (e, cx) = cx
            .add_window_view(|_, cx| Editor::new(Buffer::from_text("a\nB\nc\nd\n"), Some(PathBuf::from("x.txt")), cx));
        e.update_in(cx, |e, window, cx| {
            e.start_review("a\nb\nc\n".into(), cx);
            assert_eq!(e.review.as_ref().unwrap().hunks.len(), 2);
            // The caret starts on the first change: b became B. Keep it.
            assert_eq!(e.buffer.point(e.selection.head).0, 1);
            e.keep_hunk(&KeepHunk, window, cx);
            assert_eq!(e.review.as_ref().unwrap().hunks.len(), 1);
            // Then on the added "d": undo it, and the review is over.
            e.undo_hunk(&UndoHunk, window, cx);
            assert_eq!(e.buffer.to_string(), "a\nB\nc\n");
            assert!(!e.in_review());
        });
    }
}
