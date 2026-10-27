//! Notes on lines, in the editor (kept by `code_notes`): each under its line, quietly. ⌥⌘N
//! writes one on the caret's line (or changes the one there); Enter keeps it, Esc leaves it
//! as it was, and kept empty it goes. They move with their lines as the code is edited.

use super::{Editor, EditorEvent};
use crate::text_input::{TextInput, TextInputEvent};
use gpui::{App, AppContext, Context, Entity, Focusable, KeyBinding, Subscription, Window, actions};

actions!(notes, [AddNote, KeepNote, LeaveNote]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("alt-secondary-n", AddNote, Some("Editor")),
        KeyBinding::new("enter", KeepNote, Some("NoteDraft")),
        KeyBinding::new("escape", LeaveNote, Some("NoteDraft")),
    ]);
}

/// A note on a line of this file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineNote {
    pub id: u64,
    pub line: usize,
    pub text: String,
    pub by_ai: bool,
}

/// A note being written: on a line, new or one already there.
pub(super) struct NoteDraft {
    pub(super) line: usize,
    pub(super) id: Option<u64>,
    pub(super) input: Entity<TextInput>,
    _subscription: Subscription,
}

/// How many characters fit on a note's row, roughly, to size it.
const CHARS_PER_ROW: usize = 90;

impl Editor {
    /// The file's notes, as kept (the workspace hands them over when the file opens).
    pub fn set_line_notes(&mut self, notes: Vec<LineNote>, cx: &mut Context<Self>) {
        if self.line_notes != notes {
            self.line_notes = notes;
            self.line_notes_revision = self.buffer.revision();
            self.rebuild_blocks();
            cx.notify();
        }
    }

    pub fn line_notes(&self) -> &[LineNote] {
        &self.line_notes
    }

    /// ⌥⌘N: a note on the caret's line, or the one there to change.
    pub(super) fn add_note(&mut self, _: &AddNote, window: &mut Window, cx: &mut Context<Self>) {
        let (line, _) = self.caret_point();
        let existing = self.line_notes.iter().find(|n| n.line == line && !n.by_ai).cloned();
        self.open_note_draft(line, existing.as_ref().map(|n| (n.id, n.text.clone())), window, cx);
    }

    /// The note `id` to change (a click on it).
    pub(super) fn edit_note(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note) = self.line_notes.iter().find(|n| n.id == id).cloned() else { return };
        self.open_note_draft(note.line, Some((note.id, note.text)), window, cx);
    }

    fn open_note_draft(
        &mut self,
        line: usize,
        existing: Option<(u64, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| TextInput::new("A note on this line", cx));
        if let Some((_, text)) = &existing {
            input.update(cx, |input, cx| input.set_text_caret_at_end(text, cx));
        }
        let subscription = cx.subscribe(&input, |_, _, TextInputEvent::Changed, cx| cx.notify());
        window.focus(&input.focus_handle(cx));
        self.note_draft = Some(NoteDraft { line, id: existing.map(|(id, _)| id), input, _subscription: subscription });
        self.rebuild_blocks();
        self.autoscroll = true;
        cx.notify();
    }

    /// Enter: kept (emptied, it goes).
    pub(super) fn keep_note(&mut self, _: &KeepNote, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.note_draft.take() else { return };
        let text = draft.input.read(cx).text().trim().to_string();
        match (draft.id, text.is_empty()) {
            (Some(id), true) => self.line_notes.retain(|n| n.id != id),
            (Some(id), false) => {
                if let Some(note) = self.line_notes.iter_mut().find(|n| n.id == id) {
                    note.text = text;
                    note.by_ai = false;
                }
            }
            (None, false) => {
                self.line_notes.push(LineNote { id: crate::code_notes::new_id(), line: draft.line, text, by_ai: false })
            }
            (None, true) => {}
        }
        self.line_notes.sort_by_key(|n| n.line);
        self.line_notes_revision = self.buffer.revision();
        self.rebuild_blocks();
        window.focus(&self.focus_handle);
        cx.emit(EditorEvent::NotesChanged);
        cx.notify();
    }

    /// Esc: as it was.
    pub(super) fn leave_note(&mut self, _: &LeaveNote, window: &mut Window, cx: &mut Context<Self>) {
        self.note_draft = None;
        self.rebuild_blocks();
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// A note gone (its ✕).
    pub(super) fn remove_note(&mut self, id: u64, cx: &mut Context<Self>) {
        self.line_notes.retain(|n| n.id != id);
        self.rebuild_blocks();
        cx.emit(EditorEvent::NotesChanged);
        cx.notify();
    }

    /// After an edit (an undo is one too): the notes move with their lines.
    pub(super) fn notes_after_edit(&mut self, cx: &mut Context<Self>) {
        let revision = self.buffer.revision();
        if self.line_notes.is_empty() {
            self.line_notes_revision = revision;
            return;
        }
        let before = self.line_notes.clone();
        {
            let edits = self.buffer.edits_since(self.line_notes_revision);
            for edit in edits.into_iter().flatten() {
                for note in &mut self.line_notes {
                    note.line = super::breakpoints::move_line(note.line, edit);
                }
            }
        }
        self.line_notes_revision = revision;
        let last = self.buffer.len_lines().saturating_sub(1);
        for note in &mut self.line_notes {
            note.line = note.line.min(last);
        }
        if self.line_notes != before {
            self.rebuild_blocks();
            cx.emit(EditorEvent::NotesChanged);
        }
    }

    /// Rows a note takes under its line.
    pub(super) fn line_note_rows(note: &LineNote) -> usize {
        let rows: usize = note.text.lines().map(|l| l.chars().count().div_ceil(CHARS_PER_ROW).max(1)).sum();
        rows.clamp(1, 12)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use gpui::TestAppContext;

    #[gpui::test]
    fn notes_follow_their_lines(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(crate::settings::Settings::default()));
        let editor = cx.new(|cx| Editor::new(Buffer::from_text("a\nb\nc\n"), Some("x.rs".into()), cx));
        editor.update(cx, |e, cx| {
            e.set_line_notes(vec![LineNote { id: 1, line: 2, text: "about c".into(), by_ai: false }], cx);
            // Two lines added above it: it moves down with "c".
            e.edit(0..0, "new\nnew\n", super::super::EditKind::Other, cx);
            assert_eq!(e.line_notes()[0].line, 4);
            assert_eq!(e.buffer.line_text(4), "c");
            assert!(e.blocks.iter().any(|b| matches!(b.kind, super::super::assist::BlockKind::LineNote(0))));
        });
    }
}
