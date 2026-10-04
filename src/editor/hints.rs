//! Type hints from the language server (`x: i32`, `f(count: 3)`), shown inside the
//! code, faintly, when switched on. They stay with their code while typing and are
//! asked for again once typing pauses.

use super::Editor;
use crate::lsp_store::Readiness;
use gpui::{Context, Task};
use std::time::Duration;

/// Ask again once typing has paused this long.
const PAUSE: Duration = Duration::from_millis(400);
/// While the server starts or reads the project, look again this often.
const NOT_READY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct Hints {
    /// Byte offset in the text, and what to show there; in order.
    items: Vec<(usize, String)>,
    /// The buffer revision `items` were brought up to.
    revision: u64,
    /// The revision the server last answered for, if it's this one there's nothing to ask.
    answered: Option<u64>,
    task: Option<Task<()>>,
}

/// Where a hint goes after `edit`: along with the text before it; gone if its spot was
/// rewritten.
fn map_hint(byte: usize, edit: &crate::buffer::Edit) -> Option<usize> {
    let (start, old_end, new_end) = (edit.start_byte, edit.old_end_byte, edit.new_end_byte);
    if byte < start {
        Some(byte)
    } else if byte == start && start == old_end {
        // Typing right where the hint is: it follows what's typed (`x|: i32` → `xy|: i32`).
        Some(byte + new_end - old_end)
    } else if byte >= old_end {
        Some(byte - old_end + new_end)
    } else {
        None
    }
}

/// What a hint shows: its label, with the spaces the server asks for around it.
fn label(hint: &lsp_types::InlayHint) -> String {
    let text = match &hint.label {
        lsp_types::InlayHintLabel::String(s) => s.clone(),
        lsp_types::InlayHintLabel::LabelParts(parts) => parts.iter().map(|p| p.value.as_str()).collect(),
    };
    let left = if hint.padding_left == Some(true) { " " } else { "" };
    let right = if hint.padding_right == Some(true) { " " } else { "" };
    format!("{left}{text}{right}")
}

impl Editor {
    /// The hints on `line`: each one's column, and its text.
    pub fn hints_on_line(&self, line: usize) -> impl Iterator<Item = (usize, &str)> {
        let rope = self.buffer.rope();
        let fresh = self.hints.revision == self.buffer.revision();
        let (start, end) = if fresh && line < rope.len_lines() {
            (rope.line_to_byte(line), rope.line_to_byte((line + 1).min(rope.len_lines())))
        } else {
            (0, 0)
        };
        let from = self.hints.items.partition_point(|(b, _)| *b < start);
        let line_char = if fresh { rope.line_to_char(line.min(rope.len_lines())) } else { 0 };
        self.hints.items[from..]
            .iter()
            .take_while(move |(b, _)| *b < end || (*b == end && end == rope.len_bytes()))
            .map(move |(b, text)| (rope.byte_to_char(*b) - line_char, text.as_str()))
    }

    /// After an edit: hints move with their code, and the server is asked again later.
    pub(super) fn hints_after_edit(&mut self) {
        let revision = self.buffer.revision();
        if self.hints.revision == revision {
            return;
        }
        match self.buffer.edits_since(self.hints.revision) {
            Some(edits) => {
                let edits: Vec<_> = edits.collect();
                self.hints.items.retain_mut(|(byte, _)| match edits.iter().try_fold(*byte, |b, e| map_hint(b, e)) {
                    Some(b) => {
                        *byte = b;
                        true
                    }
                    None => false,
                });
            }
            None => self.hints.items.clear(),
        }
        self.hints.revision = revision;
        self.hints.task = None;
    }

    /// While hints are on: asks the server for this text's hints, once typing pauses.
    pub fn ensure_hints(&mut self, cx: &mut Context<Self>) {
        let revision = self.buffer.revision();
        if self.hints.task.is_some() || self.hints.answered == Some(revision) {
            return;
        }
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else { return };
        if !lsp.read(cx).has_server_for(&path) {
            return;
        }
        let end = self.lsp_position(self.buffer.len_chars());
        let range = lsp_types::Range { start: lsp_types::Position::new(0, 0), end };
        self.hints.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE).await;
            // A server still starting (or reading the project) answers with nothing yet.
            loop {
                let Ok(readiness) = this.update(cx, |this, cx| this.readiness(cx)) else { return };
                match readiness {
                    Some(Readiness::Ready { .. }) => break,
                    Some(Readiness::Starting | Readiness::Indexing { .. } | Readiness::Installing { .. }) => {
                        cx.background_executor().timer(NOT_READY).await;
                    }
                    // No server to ask (missing, or it didn't start): no hints for this text.
                    _ => {
                        this.update(cx, |this, _| this.hints.answered = Some(revision)).ok();
                        return;
                    }
                }
            }
            let Ok(request) = this.update(cx, |_, cx| lsp.read(cx).inlay_hints(&path, range)) else { return };
            let found = request.await;
            this.update(cx, |this, cx| {
                this.hints.task = None;
                if this.buffer.revision() != revision {
                    return;
                }
                let rope = this.buffer.rope().clone();
                let mut items: Vec<(usize, String)> = found
                    .iter()
                    .map(|h| (rope.char_to_byte(this.offset_from_lsp(h.position).min(rope.len_chars())), label(h)))
                    .collect();
                items.sort_by_key(|(b, _)| *b);
                this.hints = Hints { items, revision, answered: Some(revision), task: None };
                cx.notify();
            })
            .ok();
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(start: usize, old_end: usize, new_end: usize) -> crate::buffer::Edit {
        crate::buffer::Edit {
            start_byte: start,
            old_end_byte: old_end,
            new_end_byte: new_end,
            start: (0, 0),
            old_end: (0, 0),
            new_end: (0, 0),
            lsp_range: None,
            text: String::new(),
        }
    }

    #[test]
    fn hints_move_with_their_code() {
        // Typing before a hint, or right at it, carries it along.
        assert_eq!(map_hint(5, &edit(0, 0, 2)), Some(7));
        assert_eq!(map_hint(5, &edit(5, 5, 6)), Some(6));
        // After it: stays. Its spot rewritten: gone.
        assert_eq!(map_hint(5, &edit(8, 8, 9)), Some(5));
        assert_eq!(map_hint(5, &edit(3, 7, 3)), None);
        // Text deleted just before it: pulled back.
        assert_eq!(map_hint(5, &edit(3, 5, 3)), Some(3));
    }

    #[test]
    fn labels_take_their_padding() {
        let hint = |label: &str, left: bool, right: bool| lsp_types::InlayHint {
            position: lsp_types::Position::new(0, 0),
            label: lsp_types::InlayHintLabel::String(label.into()),
            kind: None,
            text_edits: None,
            tooltip: None,
            padding_left: Some(left),
            padding_right: Some(right),
            data: None,
        };
        assert_eq!(label(&hint(": i32", false, false)), ": i32");
        assert_eq!(label(&hint("count:", false, true)), "count: ");
    }
}
