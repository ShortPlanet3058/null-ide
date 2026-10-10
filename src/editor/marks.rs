//! The other uses of the symbol at the caret, tinted softly: from the language server
//! when one runs (so `count` the variable and `count` in a comment differ), else the
//! same whole word nearby. With some text selected (on one line), the same text nearby.

use super::Editor;
use gpui::{Context, Task};
use std::ops::Range;
use std::time::Duration;

/// Wait for the caret to settle before asking.
const SETTLE: Duration = Duration::from_millis(150);
/// Lines around the caret searched when there's no language server.
const NEARBY_LINES: usize = 400;
/// A selection longer than this isn't looked for elsewhere.
const SELECTION_MAX: usize = 200;

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

/// Where `word` appears in `text` (byte offsets): as a whole word, or anywhere.
fn matches(text: &str, word: &str, whole: bool) -> Vec<usize> {
    text.match_indices(word)
        .filter(|(i, _)| {
            let before = text[..*i].chars().next_back();
            let after = text[i + word.len()..].chars().next();
            !whole || (!before.is_some_and(is_word_char) && !after.is_some_and(is_word_char))
        })
        .map(|(i, _)| i)
        .collect()
}

/// Whether a selection is looked for elsewhere: on one line, not long, not only spaces.
fn worth_marking(selected: &str) -> bool {
    !selected.contains(['\n', '\r']) && !selected.trim().is_empty() && selected.chars().count() <= SELECTION_MAX
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
        if !wanted {
            return;
        }
        // Some text selected: the same text around, once the selection settles (not at
        // each step of a drag).
        if !self.selection.is_empty() {
            let range = self.selection.range();
            let selected = self.buffer.slice(range.clone());
            if !worth_marking(&selected) {
                return;
            }
            self.symbol_marks.task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SETTLE).await;
                this.update(cx, |this, cx| {
                    if this.buffer.revision() != revision || this.selection.range() != range {
                        return;
                    }
                    let found = this.nearby_uses(&selected, range.start, false);
                    // Only the selection itself: nothing to point out.
                    if found.len() < 2 {
                        return;
                    }
                    this.symbol_marks.ranges = found.into_iter().filter(|r| *r != range).collect();
                    this.symbol_marks.revision = revision;
                    cx.notify();
                })
                .ok();
            }));
            return;
        }
        if !self.extra.is_empty() {
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
                    let Ok(ranges) = this.update(cx, |this, _| this.nearby_uses(&text, word.start, true)) else { return };
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

    /// The same word (whole, or anywhere) on the lines around `at`.
    fn nearby_uses(&self, word: &str, at: usize, whole: bool) -> Vec<Range<usize>> {
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
        // Cut out of a long line, the slice's ends can be inside a word: a match there is
        // whole only if the word goes no further.
        let cut_before = start > 0 && is_word_char(rope.char(start - 1));
        let cut_after = end < rope.len_chars() && is_word_char(rope.char(end));
        matches(&text, word, whole)
            .into_iter()
            .filter(|&byte| !whole || (!(cut_before && byte == 0) && !(cut_after && byte + word.len() == text.len())))
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
        // Text selected: the same text elsewhere, inside words too, not the selection itself.
        e.update(cx, |e, cx| {
            e.selection = super::super::Selection { anchor: 10, head: 14 };
            e.refresh_symbol_marks(cx);
        });
        settle(cx);
        e.update(cx, |e, _| assert_eq!(e.symbol_marks(), [0..4, 19..23, 32..36]));
        // Over two lines, or only spaces: nothing looked for.
        e.update(cx, |e, cx| {
            e.selection = super::super::Selection { anchor: 2, head: 12 };
            e.refresh_symbol_marks(cx);
        });
        settle(cx);
        e.update(cx, |e, _| assert!(e.symbol_marks().is_empty()));
    }

    #[test]
    fn finds_whole_words_only() {
        assert_eq!(matches("count + counter + count_2 + count", "count", true), vec![0, 28]);
        assert_eq!(matches("é count", "count", true), vec![3]);
        assert_eq!(matches("count + counter", "count", false), vec![0, 8]);
        assert!(worth_marking("count") && !worth_marking("a\nb") && !worth_marking("   "));
    }
}
