//! Spelling in the editor: misspelled words in prose (Markdown, text) and in comments get
//! a faint wavy line, and ⌘. on one offers what it may have been meant to be. The word
//! being typed isn't marked until the caret leaves it, and code (in fences, in `ticks`,
//! names) never is.

use super::fixes::FixMenu;
use super::{EditKind, Editor};
use crate::theme::Syntax;
use gpui::{App, Context, ScrollHandle};
use lsp_types::{CodeActionOrCommand, Command};
use std::ops::Range;

/// Stand for picks in ⌘.'s list: spelling, handled here, not by a server.
pub(super) const CHANGE: &str = "null.spellChange";
const LEARN: &str = "null.spellLearn";
/// At most this many corrections are offered.
const GUESSES: usize = 5;

/// Which lines of a Markdown text are inside fences (``` or ~~~): code, not prose.
fn fenced_lines(rope: &ropey::Rope) -> Vec<bool> {
    let mut fence: Option<char> = None;
    // The text once, its lines as the editor counts them: no copy of each line.
    let text = rope.to_string();
    crate::markdown_view::buffer_lines(&text)
        .into_iter()
        .map(|line| {
            let start = line.trim_start_matches(' ');
            let opens = [('`', "```"), ('~', "~~~")].into_iter().find(|(_, f)| start.starts_with(f)).map(|(c, _)| c);
            match (fence, opens) {
                (Some(open), Some(c)) if c == open => {
                    fence = None;
                    true
                }
                (Some(_), _) => true,
                (None, Some(c)) => {
                    fence = Some(c);
                    true
                }
                (None, None) => false,
            }
        })
        .collect()
}

#[cfg(test)]
pub(super) fn fenced_lines_for_timing(rope: &ropey::Rope) -> Vec<bool> {
    fenced_lines(rope)
}

/// What colours the byte at `at`, if anything does.
fn syntax_at(spans: &[crate::highlight::Span], at: usize) -> Option<Syntax> {
    let i = spans.partition_point(|(r, _)| r.end <= at);
    spans.get(i).filter(|(r, _)| r.start <= at).map(|(_, s)| *s)
}

impl Editor {
    /// Whether this file's words are checked: prose, or code with comments to read.
    fn checks_spelling(&self, cx: &App) -> bool {
        cx.global::<crate::settings::Settings>().spell_check
            && self.preview.is_none()
            && (self.is_prose() || self.language().is_some())
    }

    /// Whether the line is code in a Markdown fence. Worked out once per version of the text.
    pub(super) fn in_fence(&self, line: usize) -> bool {
        if !self.is_markdown() {
            return false;
        }
        let mut fences = self.fences.borrow_mut();
        let version = self.buffer.version();
        if fences.as_ref().is_none_or(|(v, _)| *v != version) {
            *fences = Some((version, fenced_lines(self.buffer.rope())));
        }
        fences.as_ref().and_then(|(_, lines)| lines.get(line).copied()).unwrap_or(false)
    }

    /// The misspelled words of `text` (line `line`), among those checked: prose outside
    /// code, or comments.
    fn misspelled_words(&self, line: usize, text: &str, cx: &App) -> Vec<Range<usize>> {
        if !self.checks_spelling(cx) || self.in_fence(line) {
            return Vec::new();
        }
        let prose = self.is_prose();
        let line_byte = self.buffer.line_to_byte(line);
        crate::spell::misspelled_words(text, |r| {
            let syntax = syntax_at(&self.spans, line_byte + r.start);
            if prose {
                // Headings and link text are prose too; code, addresses and marks aren't.
                matches!(syntax, None | Some(Syntax::Plain | Syntax::Keyword | Syntax::Function))
            } else {
                syntax == Some(Syntax::Comment)
            }
        })
    }

    /// The misspelled words on line `line` (its text `text`), by byte range in it. The word
    /// the caret is typing at the end of waits until the caret leaves it.
    pub fn misspellings_on_line(&self, line: usize, text: &str, cx: &App) -> Vec<Range<usize>> {
        let (caret_line, caret_col) = self.caret_point();
        let typing = (caret_line == line && self.selection.is_empty())
            .then(|| text.char_indices().nth(caret_col).map_or(text.len(), |(b, _)| b));
        self.misspelled_words(line, text, cx).into_iter().filter(|r| typing != Some(r.end)).collect()
    }

    /// The misspelled word the caret is in (or just after), by char range, and the word.
    fn misspelled_at_caret(&self, cx: &App) -> Option<(Range<usize>, String)> {
        if !self.selection.is_empty() {
            return None;
        }
        let (line, column) = self.caret_point();
        let text = self.buffer.line_text(line);
        let at = text.char_indices().nth(column).map_or(text.len(), |(b, _)| b);
        let word = self.misspelled_words(line, &text, cx).into_iter().find(|r| r.start <= at && at <= r.end)?;
        let found = text[word.clone()].to_string();
        let start = self.buffer.rope().line_to_char(line) + text[..word.start].chars().count();
        Some((start..start + found.chars().count(), found))
    }

    /// ⌘. on a misspelled word: what it may have been meant to be, and Learn. True when
    /// the caret was on one.
    pub(super) fn spelling_choices(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((range, word)) = self.misspelled_at_caret(cx) else { return false };
        let command = |title: String, command: &str, word: &str| {
            CodeActionOrCommand::Command(Command {
                title,
                command: command.into(),
                arguments: Some(vec![serde_json::Value::String(word.to_string())]),
            })
        };
        let mut fixes: Vec<CodeActionOrCommand> = crate::spell::guesses(&word)
            .into_iter()
            .take(GUESSES)
            .map(|guess| command(guess.clone(), CHANGE, &guess))
            .collect();
        fixes.push(command(format!("Learn “{word}”"), LEARN, &word));
        self.close_completion(cx);
        self.close_hover(cx);
        self.fix_menu = Some(FixMenu {
            fixes,
            selected: 0,
            at: range.start,
            scroll: ScrollHandle::new(),
            conflict: None,
            spelling: Some(range),
        });
        cx.notify();
        true
    }

    /// A pick from the spelling list: the word changed, or learned.
    pub(super) fn accept_spelling(&mut self, range: Range<usize>, fix: &CodeActionOrCommand, cx: &mut Context<Self>) {
        let CodeActionOrCommand::Command(command) = fix else { return };
        let Some(word) = command.arguments.as_ref().and_then(|a| a.first()).and_then(|w| w.as_str()) else { return };
        match command.command.as_str() {
            CHANGE => {
                self.edit(range.clone(), word, EditKind::Other, cx);
                self.selection = super::Selection::caret(range.start + word.chars().count());
            }
            LEARN => crate::spell::learn(word),
            _ => {}
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fences_are_found() {
        let rope = ropey::Rope::from_str("text\n```rust\nlet teh = 1;\n```\nmore\n~~~\nx\n~~~\n");
        assert_eq!(fenced_lines(&rope), [false, true, true, true, false, true, true, true]);
    }
}
