//! The other uses of the symbol at the caret, tinted softly: from the language server
//! when one runs (so `count` the variable and `count` in a comment differ), else the
//! same whole word nearby.

use super::Editor;
use gpui::{Context, Task};
use std::ops::Range;
use std::time::Duration;

/// Wait for the caret to settle before asking.
const SETTLE: Duration = Duration::from_millis(150);
/// Lines around the caret searched when there's no language server.
const NEARBY_LINES: usize = 400;

#[derive(Default)]
pub(super) struct SymbolMarks {
    /// Char ranges, for the buffer revision below.
    ranges: Vec<Range<usize>>,
    revision: u64,
    task: Option<Task<()>>,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Where `word` appears in `text` as a whole word (byte offsets).
fn whole_word_matches(text: &str, word: &str) -> Vec<usize> {
    text.match_indices(word)
        .filter(|(i, _)| {
            let before = text[..*i].chars().next_back();
            let after = text[i + word.len()..].chars().next();
            !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
        })
        .map(|(i, _)| i)
        .collect()
}

impl Editor {
    /// The marks to draw: only for the text as it is, and not while a search shows its own.
    pub fn symbol_marks(&self) -> &[Range<usize>] {
        let marks = &self.symbol_marks;
        if marks.revision != self.buffer.revision() || self.search.is_some() {
            return &[];
        }
        &marks.ranges
    }

    /// After the caret moves: keep the marks while it stays on one of them, else ask again.
    pub(super) fn refresh_symbol_marks(&mut self, cx: &mut Context<Self>) {
        let caret = self.selection.head;
        let revision = self.buffer.revision();
        let on_mark = self.symbol_marks.revision == revision
            && self.symbol_marks.ranges.iter().any(|r| r.start <= caret && caret <= r.end);
        if on_mark && self.selection.is_empty() {
            return;
        }
        self.symbol_marks.ranges.clear();
        self.symbol_marks.task = None;
        let wanted = cx.global::<crate::settings::Settings>().symbol_marks;
        if !wanted || !self.selection.is_empty() || !self.extra.is_empty() {
            return;
        }
        let word = self.word_at(caret);
        let text = self.buffer.slice(word.clone());
        if !text.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
            return;
        }
        let server = self.lsp.clone().zip(self.path.clone()).filter(|(lsp, path)| lsp.read(cx).has_server_for(path));
        let position = self.lsp_position(caret);
        self.symbol_marks.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            let ranges = match server {
                Some((lsp, path)) => {
                    let Ok(request) = this.update(cx, |_, cx| lsp.read(cx).document_highlight(&path, position)) else {
                        return;
                    };
                    let found = request.await;
                    let Ok(ranges) = this.update(cx, |this, _| {
                        found
                            .iter()
                            .map(|h| this.offset_from_lsp(h.range.start)..this.offset_from_lsp(h.range.end))
                            .collect::<Vec<_>>()
                    }) else {
                        return;
                    };
                    ranges
                }
                None => {
                    let Ok(ranges) = this.update(cx, |this, _| this.nearby_uses(&text, word.start)) else { return };
                    ranges
                }
            };
            this.update(cx, |this, cx| {
                // One use is just the word itself: nothing to point out.
                if this.buffer.revision() != revision || ranges.len() < 2 {
                    return;
                }
                this.symbol_marks.ranges = ranges;
                this.symbol_marks.revision = revision;
                cx.notify();
            })
            .ok();
        }));
    }

    /// The same whole word on the lines around `at`.
    fn nearby_uses(&self, word: &str, at: usize) -> Vec<Range<usize>> {
        let line = self.buffer.point(at).0;
        let first = line.saturating_sub(NEARBY_LINES);
        let last = (line + NEARBY_LINES).min(self.buffer.len_lines());
        let start = self.buffer.line_to_char(first);
        let end =
            if last >= self.buffer.len_lines() { self.buffer.len_chars() } else { self.buffer.line_to_char(last) };
        // Lines, or this many characters either side on long ones (a minified file's one
        // line): matched in a slice that size.
        const AROUND: usize = 200_000;
        let (start, end) = (start.max(at.saturating_sub(AROUND)), end.min(at + AROUND));
        let text = self.buffer.slice(start..end);
        let len = word.chars().count();
        // Each match's char from the rope, not by counting from the start each time.
        let rope = self.buffer.rope();
        let start_byte = rope.char_to_byte(start);
        whole_word_matches(&text, word)
            .into_iter()
            .map(|byte| {
                let from = rope.byte_to_char(start_byte + byte);
                from..from + len
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use gpui::TestAppContext;
    use std::path::PathBuf;

    #[gpui::test]
    fn marks_the_other_uses_and_follow_the_caret(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = "count = 1\ncount += counter\nshow(count)\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("x.txt")), cx));
        let settle = |cx: &mut gpui::VisualTestContext| {
            cx.executor().advance_clock(SETTLE * 2);
            cx.run_until_parked();
        };
        e.update(cx, |e, cx| {
            e.selection = super::super::Selection::caret(2);
            e.refresh_symbol_marks(cx);
        });
        settle(cx);
        e.update(cx, |e, _| assert_eq!(e.symbol_marks(), [0..5, 10..15, 32..37]));
        // On another use: the same marks. On `counter`, used once: none.
        e.update(cx, |e, cx| {
            e.selection = super::super::Selection::caret(33);
            e.refresh_symbol_marks(cx);
            assert_eq!(e.symbol_marks().len(), 3);
            e.selection = super::super::Selection::caret(20);
            e.refresh_symbol_marks(cx);
        });
        settle(cx);
        e.update(cx, |e, _| assert!(e.symbol_marks().is_empty()));
    }

    #[test]
    fn finds_whole_words_only() {
        assert_eq!(whole_word_matches("count + counter + count_2 + count", "count"), vec![0, 28]);
        assert_eq!(whole_word_matches("é count", "count"), vec![3]);
    }
}
