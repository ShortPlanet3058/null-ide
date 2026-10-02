//! Autocomplete: suggestions from the language server in a small list under the caret.

use super::{EditKind, Editor, EditorEvent, Selection};
use crate::fuzzy;
use crate::settings::Settings;
use gpui::{Context, ScrollHandle};
use lsp_types::{CompletionItem, CompletionItemKind, CompletionTextEdit, InsertTextFormat};
use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;
use std::time::Duration;

/// Wait for a pause in typing before asking, so the list doesn't flicker.
const TYPING_PAUSE: Duration = Duration::from_millis(90);
const MAX_SHOWN: usize = 100;

pub struct Suggestion {
    pub label: String,
    pub detail: Option<String>,
    pub kind: Option<CompletionItemKind>,
    filter: String,
    sort: String,
    insert: String,
    /// Where the server wants `insert` to go, if it said.
    range: Option<lsp_types::Range>,
    extra_edits: Vec<lsp_types::TextEdit>,
}

pub struct CompletionMenu {
    suggestions: Vec<Suggestion>,
    /// Indexes into `suggestions` that match what's typed, best first, with matched bytes in the label.
    pub shown: Vec<(usize, Vec<usize>)>,
    pub selected: usize,
    /// Where the word being completed starts.
    word_start: usize,
    pub scroll: ScrollHandle,
}

impl CompletionMenu {
    /// Where the word being completed starts: the list lines up with it.
    pub fn word_start(&self) -> usize {
        self.word_start
    }

    pub fn suggestion(&self, shown_ix: usize) -> &Suggestion {
        &self.suggestions[self.shown[shown_ix].0]
    }
}

static SNIPPET_TABSTOP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$\{\d+:([^}]*)\}|\$\{\d+\}|\$\d+").unwrap());

impl Suggestion {
    fn from_lsp(item: CompletionItem) -> Self {
        let (insert, range) = match item.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => (edit.new_text, Some(edit.range)),
            Some(CompletionTextEdit::InsertAndReplace(edit)) => (edit.new_text, Some(edit.replace)),
            None => (item.insert_text.clone().unwrap_or_else(|| item.label.clone()), None),
        };
        // Placeholders aren't supported yet: keep their default text.
        let insert = if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
            SNIPPET_TABSTOP.replace_all(&insert, "$1").into_owned()
        } else {
            insert
        };
        let detail = item.label_details.as_ref().and_then(|d| d.description.clone()).or(item.detail.clone());
        Self {
            filter: item.filter_text.clone().unwrap_or_else(|| item.label.clone()),
            sort: item.sort_text.clone().unwrap_or_else(|| item.label.clone()),
            label: item.label,
            detail,
            kind: item.kind,
            insert,
            range,
            extra_edits: item.additional_text_edits.unwrap_or_default(),
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Editor {
    /// After typing `text`: open, refresh or close the list.
    pub(super) fn completion_after_typing(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.lsp.is_none() || !self.selection.is_empty() || self.multi_cursor() {
            return self.close_completion(cx);
        }
        let Some(last) = text.chars().last() else { return };
        let caret = self.selection.head;
        let previous = caret.checked_sub(2).and_then(|i| self.buffer.char_at(i));
        let trigger = match last {
            '.' => Some('.'),
            ':' if previous == Some(':') => Some(':'),
            _ => None,
        };
        if trigger.is_none() && !is_word_char(last) {
            return self.close_completion(cx);
        }
        if self.completion.is_none() && !cx.global::<Settings>().autocomplete {
            return;
        }
        let word_start = if trigger.is_some() { caret } else { self.word_start_before(caret) };
        self.request_completion(word_start, trigger, TYPING_PAUSE, cx);
    }

    /// After deleting: keep the list in step, or close it once the word is gone.
    pub(super) fn completion_after_delete(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = &self.completion else { return };
        let word_start = menu.word_start;
        let caret = self.selection.head;
        // Close once the word is gone, unless the caret sits right after a `.` or `::`.
        if caret < word_start || caret == word_start && !self.after_trigger(word_start) {
            return self.close_completion(cx);
        }
        self.refilter(cx);
        self.request_completion(word_start, None, TYPING_PAUSE, cx);
    }

    fn after_trigger(&self, offset: usize) -> bool {
        matches!(offset.checked_sub(1).and_then(|i| self.buffer.char_at(i)), Some('.' | ':'))
    }

    fn word_start_before(&self, offset: usize) -> usize {
        let mut start = offset;
        while start > 0 && self.buffer.char_at(start - 1).is_some_and(is_word_char) {
            start -= 1;
        }
        start
    }

    /// Ctrl+Space: ask for suggestions now, even with autocomplete off.
    pub(super) fn show_completions_now(&mut self, cx: &mut Context<Self>) {
        let caret = self.selection.head;
        self.request_completion(self.word_start_before(caret), None, Duration::ZERO, cx);
    }

    fn request_completion(
        &mut self,
        word_start: usize,
        trigger: Option<char>,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else { return };
        let position = self.lsp_position(self.selection.head);
        self.completion_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(request) = this.update(cx, |_, cx| lsp.read(cx).completion(&path, position, trigger)) else {
                return;
            };
            let items = request.await;
            this.update(cx, |this, cx| {
                if items.is_empty() {
                    return this.close_completion(cx);
                }
                let selected_label = this
                    .completion
                    .as_ref()
                    .and_then(|m| m.shown.get(m.selected))
                    .map(|&(i, _)| this.completion.as_ref().unwrap().suggestions[i].label.clone());
                let scroll = this.completion.take().map(|m| m.scroll).unwrap_or_default();
                this.completion = Some(CompletionMenu {
                    suggestions: items.into_iter().map(Suggestion::from_lsp).collect(),
                    shown: Vec::new(),
                    selected: 0,
                    word_start,
                    scroll,
                });
                this.refilter(cx);
                // Keep the same suggestion selected when the list refreshes under it.
                if let (Some(label), Some(menu)) = (selected_label, this.completion.as_mut())
                    && let Some(ix) = menu.shown.iter().position(|&(i, _)| menu.suggestions[i].label == label)
                {
                    menu.selected = ix;
                }
            })
            .ok();
        }));
    }

    /// Re-ranks the suggestions against what's typed so far.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        let caret = self.selection.head;
        let Some(menu) = &mut self.completion else { return };
        let typed = self.buffer.slice(menu.word_start..caret);
        let mut shown: Vec<(usize, i32, Vec<usize>)> = menu
            .suggestions
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let (score, _) = fuzzy::score(&s.filter, &typed)?;
                let highlights = fuzzy::score(&s.label, &typed).map(|(_, h)| h).unwrap_or_default();
                Some((i, score, highlights))
            })
            .collect();
        shown.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| menu.suggestions[a.0].sort.cmp(&menu.suggestions[b.0].sort)));
        shown.truncate(MAX_SHOWN);
        menu.shown = shown.into_iter().map(|(i, _, h)| (i, h)).collect();
        menu.selected = 0;
        menu.scroll.scroll_to_item(0);
        if menu.shown.is_empty() {
            self.completion = None;
        }
        cx.notify();
    }

    pub(super) fn close_completion(&mut self, cx: &mut Context<Self>) {
        self.completion_task = None;
        if self.completion.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn move_completion(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(menu) = &mut self.completion else { return };
        let len = menu.shown.len() as isize;
        menu.selected = (menu.selected as isize + delta).rem_euclid(len) as usize;
        menu.scroll.scroll_to_item(menu.selected);
        cx.notify();
    }

    /// Inserts the chosen suggestion, plus any edits it brings along (like an import).
    pub(super) fn accept_completion(&mut self, shown_ix: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.completion.take() else { return };
        self.completion_task = None;
        let Some(&(ix, _)) = menu.shown.get(shown_ix) else { return };
        let suggestion = &menu.suggestions[ix];
        let caret = self.selection.head;
        // The server's range was computed before the latest keystrokes: stretch it to the caret.
        let start = suggestion.range.map_or(menu.word_start, |r| self.offset_from_lsp(r.start).min(caret));
        // Accepting in the middle of a word replaces the rest of it too (fo|obar → foobar).
        let end = suggestion.range.map_or(caret, |r| self.offset_from_lsp(r.end).max(caret));
        let mut edits: Vec<(Range<usize>, String)> = vec![(start..end, suggestion.insert.clone())];
        for edit in &suggestion.extra_edits {
            let range = self.offset_from_lsp(edit.range.start)..self.offset_from_lsp(edit.range.end);
            if range.end <= start || range.start >= caret {
                edits.push((range, edit.new_text.clone()));
            }
        }
        // Where the caret lands: after the inserted text, shifted by edits above it.
        let shift: isize = edits[1..]
            .iter()
            .filter(|(r, _)| r.end <= start)
            .map(|(r, t)| t.chars().count() as isize - r.len() as isize)
            .sum();
        let landing = (start as isize + shift) as usize + suggestion.insert.chars().count();

        self.record_undo(EditKind::Other);
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        for (range, text) in edits {
            self.buffer.replace(range, &text);
        }
        self.single_cursor();
        self.selection = Selection::caret(landing);
        self.marked = None;
        self.goal_column = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, TextEdit};

    #[test]
    fn snippets_keep_their_default_text() {
        let item = CompletionItem {
            label: "push(…)".into(),
            insert_text: Some("push(${1:value})$0".into()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        };
        let s = Suggestion::from_lsp(item);
        assert_eq!(s.insert, "push(value)");
        assert_eq!(s.filter, "push(…)");
    }

    #[test]
    fn uses_the_servers_edit_and_details() {
        let range = lsp_types::Range { start: Position::new(0, 4), end: Position::new(0, 6) };
        let item = CompletionItem {
            label: "len".into(),
            detail: Some("fn(&self) -> usize".into()),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: "len()".into() })),
            sort_text: Some("0001".into()),
            ..Default::default()
        };
        let s = Suggestion::from_lsp(item);
        assert_eq!((s.insert.as_str(), s.range), ("len()", Some(range)));
        assert_eq!(s.detail.as_deref(), Some("fn(&self) -> usize"));
        assert_eq!(s.sort, "0001");
    }
}
