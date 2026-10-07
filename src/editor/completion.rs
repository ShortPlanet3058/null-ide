//! Autocomplete: suggestions from the language server in a small list under the caret.

use super::{EditKind, Editor, EditorEvent, Selection};
use crate::fuzzy;
use crate::settings::Settings;
use gpui::{Context, ScrollHandle};
use lsp_types::{CompletionItem, CompletionItemKind, CompletionTextEdit, InsertTextFormat};
use std::ops::Range;
use std::time::Duration;

/// Wait for a pause in typing before asking, so the list doesn't flicker.
const TYPING_PAUSE: Duration = Duration::from_millis(90);
const MAX_SHOWN: usize = 100;
/// Words of the file are looked for this many lines around the caret, and no further
/// than this many characters (a minified file is one long line).
const WORDS_AROUND: usize = 2000;
const WORDS_AROUND_CHARS: usize = 100_000;
/// Without a language server, the list opens by itself once a word is this long.
const WORDS_AFTER: usize = 2;

pub struct Suggestion {
    pub label: String,
    pub detail: Option<String>,
    pub kind: Option<CompletionItemKind>,
    filter: String,
    sort: String,
    insert: String,
    /// `insert` is a snippet, with placeholders to fill in.
    snippet: bool,
    /// Where the server wants `insert` to go, if it said.
    range: Option<lsp_types::Range>,
    extra_edits: Vec<lsp_types::TextEdit>,
    /// The item as the server sent it, when it can say more once picked (the import it adds).
    unresolved: Option<CompletionItem>,
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

impl Suggestion {
    fn from_lsp(item: CompletionItem) -> Self {
        let unresolved = (item.additional_text_edits.is_none() && item.data.is_some()).then(|| item.clone());
        let (insert, range) = match item.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => (edit.new_text, Some(edit.range)),
            Some(CompletionTextEdit::InsertAndReplace(edit)) => (edit.new_text, Some(edit.replace)),
            None => (item.insert_text.clone().unwrap_or_else(|| item.label.clone()), None),
        };
        let snippet = item.insert_text_format == Some(InsertTextFormat::SNIPPET);
        let detail = item.label_details.as_ref().and_then(|d| d.description.clone()).or(item.detail.clone());
        Self {
            filter: item.filter_text.clone().unwrap_or_else(|| item.label.clone()),
            sort: item.sort_text.clone().unwrap_or_else(|| item.label.clone()),
            label: item.label,
            detail,
            kind: item.kind,
            insert,
            snippet,
            range,
            extra_edits: item.additional_text_edits.unwrap_or_default(),
            unresolved,
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Folders listed this many entries at most (a huge folder of build output).
const MAX_ENTRIES: usize = 500;
/// How many folders up from the file a relative path is looked for, at most: up to the
/// project's top (where `.git` is).
const FOLDERS_UP: usize = 8;

/// The path written at the end of `before` (the line up to the caret): the folder part as
/// written (up to its last `/`) and the name being typed after it. Only what reads as a
/// path: not `a / b`, a `//` comment, a web address, nor `/` alone.
fn path_at_end(before: &str) -> Option<(&str, &str)> {
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace() || "\"'`([<>{}=,;|".contains(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let token = &before[start..];
    let slash = token.rfind('/')?;
    let (dir, name) = (&token[..=slash], &token[slash + 1..]);
    if token.contains("//") || token.contains("/*") || token.contains(':') {
        return None;
    }
    // An absolute path only past its first folder (`/Users/`), so a division sign or a
    // regex's `/` doesn't list the disk.
    if dir.starts_with('/') && dir.matches('/').count() < 2 {
        return None;
    }
    Some((dir, name))
}

/// A Markdown link's `#section` being written at the end of `before` (`](#ge`,
/// `](guide.md#ge`): the file as written (empty for this one) and what's typed after `#`.
fn anchor_at_end(before: &str) -> Option<(&str, &str)> {
    let hash = before.rfind('#')?;
    let typed = &before[hash + 1..];
    if !typed.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let open = before[..hash].rfind("](")?;
    let file = &before[open + 2..hash];
    (!file.contains([' ', '(', ')']) && !file.contains("://")).then_some((file, typed))
}

/// The folder `written` names, from the file in `dir`: there, or in a folder above it up to
/// the project's top (`.git`). Outside a repository, only from the file's own folder: the
/// list opens by itself, so not from any folder that happens to be above.
fn folder_for(written: &str, dir: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let path = match (written.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => std::path::PathBuf::from(written),
    };
    if path.is_absolute() {
        return path.is_dir().then_some(path);
    }
    let dir = dir?;
    let top = dir.ancestors().take(FOLDERS_UP).position(|d| d.join(".git").exists()).unwrap_or(0);
    dir.ancestors().take(top + 1).map(|d| d.join(&path)).find(|p| p.is_dir())
}

/// What's in `folder`, as suggestions: folders first (with their `/`), hidden ones only
/// once a `.` is typed.
fn entries(folder: &std::path::Path, typed: &str) -> Vec<CompletionItem> {
    let Ok(read) = std::fs::read_dir(folder) else { return Vec::new() };
    read.filter_map(Result::ok)
        .take(MAX_ENTRIES)
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if name.starts_with('.') && !typed.starts_with('.') {
                return None;
            }
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            let label = if is_dir { format!("{name}/") } else { name.clone() };
            Some(CompletionItem {
                sort_text: Some(format!("{}{name}", if is_dir { 0 } else { 1 })),
                filter_text: Some(name),
                kind: Some(if is_dir { CompletionItemKind::FOLDER } else { CompletionItemKind::FILE }),
                label,
                ..Default::default()
            })
        })
        .collect()
}

impl Editor {
    /// After typing `text`: open, refresh or close the list.
    pub(super) fn completion_after_typing(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.selection.is_empty() || self.multi_cursor() {
            return self.close_completion(cx);
        }
        // A path being written (`./`, `src/`, `img/`): what's in that folder.
        if (self.completion.is_some() || cx.global::<Settings>().autocomplete)
            && (self.complete_anchor(cx) || self.complete_path(cx))
        {
            return;
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
        // The file's own words: they open by themselves in code, not while writing prose, nor
        // when AI completions already suggest the file's names inline.
        let own_words = !self.served(cx);
        let quiet = self.is_prose() || caret - word_start < WORDS_AFTER || cx.global::<Settings>().ai.completions;
        if own_words && self.completion.is_none() && quiet {
            return;
        }
        self.request_completion(word_start, trigger, TYPING_PAUSE, cx);
    }

    /// Whether a language server gives this file's suggestions (otherwise its own words do).
    pub(super) fn served(&self, cx: &Context<Self>) -> bool {
        match (&self.lsp, &self.path) {
            (Some(lsp), Some(path)) => lsp.read(cx).serves(path),
            _ => false,
        }
    }

    /// When the caret ends a path to a folder that exists, lists what's in it and returns
    /// true; otherwise false, and nothing changes.
    fn complete_path(&mut self, cx: &mut Context<Self>) -> bool {
        let caret = self.selection.head;
        let (line, column) = self.buffer.point(caret);
        let text = self.buffer.line_text(line);
        let before: String = text.chars().take(column).collect();
        let Some((dir, name)) = path_at_end(&before) else { return false };
        let Some(folder) = folder_for(dir, self.path.as_deref().and_then(std::path::Path::parent)) else {
            return false;
        };
        let items = entries(&folder, name);
        if items.is_empty() {
            return false;
        }
        let name_start = caret - name.chars().count();
        self.completion_task = None;
        self.show_suggestions(items, name_start, cx);
        self.completion.is_some()
    }

    /// In Markdown, when the caret ends a link's `#section` (this file's or another's): its
    /// headings, by their anchors. True when it listed them.
    fn complete_anchor(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.is_markdown() {
            return false;
        }
        let caret = self.selection.head;
        let (line, column) = self.buffer.point(caret);
        let text = self.buffer.line_text(line);
        let before: String = text.chars().take(column).collect();
        let Some((file, typed)) = anchor_at_end(&before) else { return false };
        let source = if file.is_empty() {
            self.buffer.to_string()
        } else {
            let Some(dir) = self.path.as_deref().and_then(std::path::Path::parent) else { return false };
            let path = crate::markdown_links::tidy(&dir.join(file.replace("%20", " ")));
            let markdown = path.extension().is_some_and(|x| x == "md" || x == "markdown" || x == "mdx");
            // Read as the caret moves: not a huge file.
            let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= 1024 * 1024);
            match std::fs::read_to_string(&path) {
                Ok(text) if markdown && small => text,
                _ => return false,
            }
        };
        let items: Vec<CompletionItem> = crate::markdown_view::headings(&source)
            .into_iter()
            .enumerate()
            .map(|(i, (_, heading, anchor))| CompletionItem {
                sort_text: Some(format!("{i:05}")),
                filter_text: Some(anchor.clone()),
                kind: Some(CompletionItemKind::REFERENCE),
                detail: Some(heading),
                label: anchor,
                ..Default::default()
            })
            .collect();
        if items.is_empty() {
            return false;
        }
        self.completion_task = None;
        self.show_suggestions(items, caret - typed.chars().count(), cx);
        self.completion.is_some()
    }

    /// The words of the file around the caret, nearest first, but the one being typed.
    fn words_near(&self, word_start: usize) -> Vec<CompletionItem> {
        let caret = self.selection.head;
        let line = self.buffer.point(caret).0;
        let first =
            self.buffer.line_to_char(line.saturating_sub(WORDS_AROUND)).max(caret.saturating_sub(WORDS_AROUND_CHARS));
        let last = self.buffer.line_to_char(line + WORDS_AROUND).min(caret + WORDS_AROUND_CHARS);
        let mut nearest: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut word = String::new();
        let mut start = first;
        let text = self.buffer.slice(first..last);
        for (i, c) in text.chars().chain([' ']).enumerate() {
            let at = first + i;
            if is_word_char(c) {
                if word.is_empty() {
                    start = at;
                }
                word.push(c);
                continue;
            }
            // Not the word being typed itself.
            if word.chars().count() >= 3 && !word.starts_with(|c: char| c.is_ascii_digit()) && start != word_start {
                let distance = if at <= word_start { word_start - at } else { start - caret };
                let best = nearest.entry(std::mem::take(&mut word)).or_insert(distance);
                *best = (*best).min(distance);
            }
            word.clear();
        }
        nearest
            .into_iter()
            .map(|(label, distance)| CompletionItem {
                sort_text: Some(format!("{distance:010}")),
                kind: Some(CompletionItemKind::TEXT),
                label,
                ..Default::default()
            })
            .collect()
    }

    /// After deleting: keep the list in step, or close it once the word is gone.
    pub(super) fn completion_after_delete(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = &self.completion else { return };
        let word_start = menu.word_start;
        let caret = self.selection.head;
        // Close once the word is gone, unless the caret sits right after a `.` or `::`.
        // Still in a path (or a link's #section): what's there, for what's left of the name.
        if self.complete_anchor(cx) || self.complete_path(cx) {
            return;
        }
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
        if self.complete_anchor(cx) || self.complete_path(cx) {
            return;
        }
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
        if !self.served(cx) {
            let words = self.words_near(word_start);
            return self.show_suggestions(words, word_start, cx);
        }
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else { return };
        let position = self.lsp_position(self.selection.head);
        self.completion_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(request) = this.update(cx, |_, cx| lsp.read(cx).completion(&path, position, trigger)) else {
                return;
            };
            let items = request.await;
            this.update(cx, |this, cx| this.show_suggestions(items, word_start, cx)).ok();
        }));
    }

    /// Shows `items` as the list for the word starting at `word_start`, keeping the selected
    /// suggestion when the list refreshes under it.
    fn show_suggestions(&mut self, items: Vec<CompletionItem>, word_start: usize, cx: &mut Context<Self>) {
        if items.is_empty() {
            return self.close_completion(cx);
        }
        let selected_label = self
            .completion
            .as_ref()
            .and_then(|m| m.shown.get(m.selected).map(|&(i, _)| m.suggestions[i].label.clone()));
        let scroll = self.completion.take().map(|m| m.scroll).unwrap_or_default();
        self.completion = Some(CompletionMenu {
            suggestions: items.into_iter().map(Suggestion::from_lsp).collect(),
            shown: Vec::new(),
            selected: 0,
            word_start,
            scroll,
        });
        self.refilter(cx);
        if let (Some(label), Some(menu)) = (selected_label, self.completion.as_mut())
            && let Some(ix) = menu.shown.iter().position(|&(i, _)| menu.suggestions[i].label == label)
        {
            menu.selected = ix;
        }
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
        self.close_fixes(cx);
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
        let parsed = suggestion.snippet.then(|| super::snippet::parse(&suggestion.insert));
        let text = parsed.as_ref().map_or_else(|| suggestion.insert.clone(), |p| p.text.clone());
        let mut edits: Vec<(Range<usize>, String)> = vec![(start..end, text.clone())];
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
        let inserted_at = (start as isize + shift) as usize;
        let landing = inserted_at + text.chars().count();

        self.record_undo(EditKind::Other);
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        for (range, text) in edits {
            self.buffer.replace(range, &text);
        }
        self.single_cursor();
        self.selection = Selection::caret(landing);
        self.marked = None;
        self.goal_column = None;
        if let Some(parsed) = parsed {
            self.start_snippet(inserted_at, parsed, cx);
        }
        if let Some(item) = suggestion.unresolved.clone() {
            self.resolve_picked(item, inserted_at, cx);
        }
        // A folder picked: on into it.
        let into_folder = suggestion.kind == Some(CompletionItemKind::FOLDER) && suggestion.insert.ends_with('/');
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
        if into_folder {
            self.complete_path(cx);
        }
    }

    /// Asks the server what else a picked completion brings (the import it adds), then adds
    /// that above it, as long as nothing above was changed in the meantime.
    fn resolve_picked(&mut self, item: CompletionItem, inserted_at: usize, cx: &mut Context<Self>) {
        let (Some(lsp), Some(path)) = (self.lsp.clone().filter(|_| !self.lsp_follower), self.path.clone()) else {
            return;
        };
        let request = lsp.read(cx).resolve_completion(&path, item);
        let before = self.lsp_position(inserted_at);
        let before_byte = self.buffer.rope().char_to_byte(inserted_at);
        let revision = self.buffer.revision();
        cx.spawn(async move |this, cx| {
            let Ok(resolved) = request.await else { return };
            let edits: Vec<lsp_types::TextEdit> = resolved
                .additional_text_edits
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.range.end <= before)
                .collect();
            if edits.is_empty() {
                return;
            }
            this.update(cx, |this, cx| {
                let untouched =
                    this.buffer.edits_since(revision).is_some_and(|mut e| e.all(|e| e.start_byte >= before_byte));
                if untouched {
                    this.add_edits_above(&edits, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// A server's edits, as one undo step, the cursors (and what they select) staying with
    /// their text. Nothing changes when an edit would rewrite text a cursor is in.
    fn add_edits_above(&mut self, edits: &[lsp_types::TextEdit], cx: &mut Context<Self>) {
        let ranges = super::refactor::edit_ranges(&self.buffer, edits);
        let moved = |s: Selection| {
            let anchor = super::refactor::caret_after(s.anchor, &ranges)?;
            Some(Selection { anchor, head: super::refactor::caret_after(s.head, &ranges)? })
        };
        let Some(selection) = moved(self.selection) else { return };
        let Some(extra) = self
            .extra
            .iter()
            .map(|c| moved(c.selection).map(|selection| super::Cursor { selection, goal: None }))
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        self.record_undo(EditKind::Other);
        super::refactor::apply_edits(&mut self.buffer, edits);
        self.selection = selection;
        self.extra = extra;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.touch(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_are_told_from_links() {
        assert_eq!(anchor_at_end("See [a](#ge"), Some(("", "ge")));
        assert_eq!(anchor_at_end("[a](guide.md#"), Some(("guide.md", "")));
        assert_eq!(anchor_at_end("# heading"), None);
        assert_eq!(anchor_at_end("[a](https://x.dev/#top"), None);
    }

    #[test]
    fn paths_are_told_from_code() {
        assert_eq!(path_at_end("![shot](img/sh"), Some(("img/", "sh")));
        assert_eq!(path_at_end("<img src=\"./assets/"), Some(("./assets/", "")));
        assert_eq!(path_at_end("import x from '../lib/ut"), Some(("../lib/", "ut")));
        assert_eq!(path_at_end("cat ~/Documents/no"), Some(("~/Documents/", "no")));
        assert_eq!(path_at_end("let half = total/"), Some(("total/", "")), "a folder only if one exists");
        assert_eq!(path_at_end("x = a / b"), None);
        assert_eq!(path_at_end("// note"), None);
        assert_eq!(path_at_end("see https://x.dev/do"), None);
        assert_eq!(path_at_end("a = b /"), None, "the disk's root isn't listed");
    }
    use lsp_types::{Position, TextEdit};

    #[test]
    fn items_the_server_completes_later_are_kept_to_ask_about() {
        let item = |data: Option<serde_json::Value>, edits: Option<Vec<TextEdit>>| CompletionItem {
            label: "useState".into(),
            data,
            additional_text_edits: edits,
            ..Default::default()
        };
        assert!(Suggestion::from_lsp(item(Some(serde_json::json!({"id": 1})), None)).unresolved.is_some());
        // Already complete, or nothing to ask about with.
        assert!(Suggestion::from_lsp(item(Some(serde_json::json!(1)), Some(Vec::new()))).unresolved.is_none());
        assert!(Suggestion::from_lsp(item(None, None)).unresolved.is_none());
    }

    /// The import a picked completion brings arrives once the placeholder is selected:
    /// it goes in above, the placeholder stays selected, and ⌘Z takes back just the import.
    #[gpui::test]
    fn an_import_arriving_later_leaves_the_placeholder_selected(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        use gpui::Focusable;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(Buffer::from_text("const a = \n"), Some(std::path::PathBuf::from("x.ts")), cx)
        });
        let selected = |e: &Editor| e.buffer.slice(e.selection.range());
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            let parsed = super::super::snippet::parse("useState(${1:initial})$0");
            e.buffer.replace(10..10, &parsed.text);
            e.start_snippet(10, parsed, cx);
            assert_eq!(selected(e), "initial");
            let import = TextEdit {
                range: lsp_types::Range { start: Position::new(0, 0), end: Position::new(0, 0) },
                new_text: "import { useState } from \"react\";\n".into(),
            };
            e.add_edits_above(&[import], cx);
            assert_eq!(e.buffer.to_string(), "import { useState } from \"react\";\nconst a = useState(initial)\n");
            assert_eq!(selected(e), "initial");
        });
        cx.simulate_input("0");
        cx.simulate_keystrokes("tab");
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "import { useState } from \"react\";\nconst a = useState(0)\n");
            assert!(!e.in_snippet());
        });
        cx.simulate_keystrokes("cmd-z cmd-z");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "const a = useState(initial)\n"));
    }

    fn editor<'a>(
        cx: &'a mut gpui::TestAppContext,
        name: &str,
        text: &str,
    ) -> (gpui::Entity<Editor>, &'a mut gpui::VisualTestContext) {
        use gpui::Focusable;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let (name, text) = (std::path::PathBuf::from(name), text.to_string());
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(crate::buffer::Buffer::from_text(&text), Some(name), cx));
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            e.selection = Selection::caret(e.buffer.len_chars());
        });
        (e, cx)
    }

    fn shown(cx: &mut gpui::VisualTestContext, e: &gpui::Entity<Editor>) -> Vec<String> {
        e.update(cx, |e, _| {
            e.completion
                .as_ref()
                .map_or(Vec::new(), |m| (0..m.shown.len()).map(|i| m.suggestion(i).label.clone()).collect())
        })
    }

    /// With no language server, the file's own words: nearest first, not the one typed,
    /// once two letters are in.
    #[gpui::test]
    fn without_a_server_the_files_words_are_suggested(cx: &mut gpui::TestAppContext) {
        let (e, cx) = editor(cx, "deploy.yaml", "release_tag: 1\nname: x\nrelease_name: y\nr");
        cx.run_until_parked();
        assert!(shown(cx, &e).is_empty());
        cx.simulate_input("e");
        cx.run_until_parked();
        assert_eq!(shown(cx, &e), ["release_name", "release_tag"]);
        cx.simulate_input("l");
        cx.simulate_keystrokes("enter");
        e.update(cx, |e, _| assert!(e.buffer.to_string().ends_with("release_name: y\nrelease_name")));
    }

    /// A link's #section lists the headings' anchors: this file's, or the file named's.
    #[gpui::test]
    fn sections_are_completed_from_headings(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("anchor-completion");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("other.md"), "# Other\n## Install it\n").unwrap();
        let notes = dir.join("notes.md");
        let (e, cx) = editor(cx, notes.to_str().unwrap(), "# Guide\n## Get started\n## Keys\nSee [s](");
        cx.simulate_input("#");
        cx.run_until_parked();
        assert_eq!(shown(cx, &e), ["guide", "get-started", "keys"]);
        cx.simulate_input("ke");
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        e.update(cx, |e, _| assert!(e.buffer.to_string().ends_with("See [s](#keys")));
        cx.simulate_input(") and [o](other.md#");
        cx.run_until_parked();
        assert_eq!(shown(cx, &e), ["other", "install-it"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Writing a path lists what's in its folder: a folder picked goes on into it, a file
    /// picked finishes the path.
    #[gpui::test]
    fn paths_are_completed_from_the_folder(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("path-completion");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("img/icons")).unwrap();
        for f in ["img/shot.png", "img/hero.jpg", "img/.hidden"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let notes = dir.join("notes.md");
        let (e, cx) = editor(cx, notes.to_str().unwrap(), "See ![a](");
        cx.simulate_input("img/");
        cx.run_until_parked();
        assert_eq!(shown(cx, &e), ["icons/", "hero.jpg", "shot.png"], "folders first, hidden ones not");
        cx.simulate_input("sh");
        cx.run_until_parked();
        assert_eq!(shown(cx, &e), ["shot.png"]);
        cx.simulate_keystrokes("enter");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "See ![a](img/shot.png"));
        // Code with a division isn't a path.
        cx.simulate_input(" x/2");
        cx.run_until_parked();
        assert!(shown(cx, &e).is_empty(), "{:?} for {:?}", shown(cx, &e), e.read_with(cx, |e, _| e.buffer.to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// In prose they only come when asked for: writing doesn't pop a list up.
    #[gpui::test]
    fn prose_gets_words_only_when_asked(cx: &mut gpui::TestAppContext) {
        let (e, cx) = editor(cx, "notes.md", "Paragraphs and paragons.\n\npar");
        cx.simulate_input("a");
        cx.run_until_parked();
        assert!(shown(cx, &e).is_empty());
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        assert_eq!(shown(cx, &e), ["paragons", "Paragraphs"]);
    }

    #[test]
    fn snippets_are_kept_to_fill_in() {
        let item = CompletionItem {
            label: "push(…)".into(),
            insert_text: Some("push(${1:value})$0".into()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        };
        let s = Suggestion::from_lsp(item);
        assert_eq!((s.insert.as_str(), s.snippet), ("push(${1:value})$0", true));
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
