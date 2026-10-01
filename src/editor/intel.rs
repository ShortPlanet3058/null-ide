//! Code intelligence for the editor, backed by a language server: diagnostics,
//! hover cards (hold Alt/Option) and go to definition.

use super::{Editor, EditorEvent, Selection};
use crate::lsp::path_for;
use crate::lsp_store::{LspStore, Readiness};
use gpui::{App, Context, Entity};
use lsp_types::{DiagnosticSeverity, HoverContents, MarkedString, Position};
use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;
use std::time::Duration;

/// Alt over a word this long shows its card.
const HOVER_DELAY: Duration = Duration::from_millis(80);
/// With a card open, the mouse has to rest on another word this long to switch to it.
const HOVER_SWITCH_DELAY: Duration = Duration::from_millis(350);
/// How long the mouse can be away from the word and the card before the card closes.
const HOVER_GRACE: Duration = Duration::from_millis(450);
/// Cards show the signature and the start of the docs, not whole READMEs.
const MAX_HOVER_LINES: usize = 14;

pub struct HoverCard {
    pub range: Range<usize>,
    pub diagnostics: Vec<(DiagnosticSeverity, String)>,
    pub blocks: Vec<HoverBlock>,
}

pub struct HoverBlock {
    pub code: bool,
    pub text: String,
}

/// A problem reported by the language server, in char offsets.
pub struct Problem {
    pub range: Range<usize>,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

impl Editor {
    pub(super) fn attach_lsp(&mut self, lsp: Entity<LspStore>, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return };
        if !lsp.read(cx).has_server_for(&path) {
            return;
        }
        let text = self.buffer.to_string();
        lsp.update(cx, |lsp, cx| lsp.open(&path, text, cx));
        self.lsp_subscription = Some(cx.observe(&lsp, |_, _, cx| cx.notify()));
        self.lsp = Some(lsp);
    }

    /// Re-sends the file after the project's language servers restarted.
    pub fn reattach_lsp(&mut self, cx: &mut Context<Self>) {
        self.lsp_version = 0;
        if let Some(lsp) = self.lsp.take() {
            self.attach_lsp(lsp, cx);
        }
    }

    /// Tells the server the file is no longer open. Call before dropping the editor.
    pub fn release_lsp(&mut self, cx: &mut Context<Self>) {
        if let (Some(lsp), Some(path)) = (self.lsp.take(), self.path.clone()) {
            lsp.update(cx, |lsp, cx| lsp.close(&path, cx));
        }
        self.lsp_subscription = None;
    }

    pub(super) fn sync_lsp(&mut self, cx: &mut Context<Self>) {
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else { return };
        self.lsp_version += 1;
        let (text, version) = (self.buffer.to_string(), self.lsp_version);
        lsp.update(cx, |lsp, cx| lsp.change(&path, text, version, cx));
    }

    pub(super) fn lsp_saved(&mut self, cx: &mut Context<Self>) {
        if let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) {
            lsp.update(cx, |lsp, cx| lsp.save(&path, cx));
        }
    }

    fn lsp_position(&self, offset: usize) -> Position {
        let (line, column) = self.buffer.point(offset);
        Position { line: line as u32, character: self.buffer.column_to_utf16(line, column) as u32 }
    }

    pub fn offset_from_lsp(&self, position: Position) -> usize {
        let line = position.line as usize;
        self.buffer.offset(line, self.buffer.utf16_to_column(line, position.character as usize))
    }

    /// Errors, warnings and notes the server reported for this file.
    pub fn problems(&self, cx: &App) -> Vec<Problem> {
        let (Some(lsp), Some(path)) = (&self.lsp, &self.path) else { return Vec::new() };
        lsp.read(cx)
            .diagnostics(path)
            .iter()
            .map(|d| Problem {
                range: self.offset_from_lsp(d.range.start)..self.offset_from_lsp(d.range.end),
                severity: d.severity.unwrap_or(DiagnosticSeverity::ERROR),
                message: d.message.clone(),
            })
            .collect()
    }

    /// Shows, moves or hides the hover card to match the mouse and the Alt key.
    pub fn readiness(&self, cx: &App) -> Option<Readiness> {
        self.lsp.as_ref()?.read(cx).readiness(self.path.as_ref()?)
    }

    /// Why hover and go to definition can't answer yet, in words.
    fn not_ready_message(&self, cx: &App) -> Option<String> {
        let label = LspStore::language_label(self.path.as_ref()?).unwrap_or("Code");
        match self.readiness(cx)? {
            Readiness::Ready { .. } => None,
            Readiness::Starting => Some(format!("Starting {label} support…")),
            Readiness::Indexing { percent } => {
                let percent = percent.map(|p| format!(" ({p}%)")).unwrap_or_default();
                Some(format!(
                    "{label} support is still reading this project{percent}. Info shows up here once it's done."
                ))
            }
            Readiness::Unavailable { program } => Some(format!("Install {program} to get info here.")),
        }
    }

    /// Shows a short message in the hover card spot for a few seconds.
    fn show_notice(&mut self, offset: usize, message: String, cx: &mut Context<Self>) {
        let word = self.word_at(offset);
        self.hover_word = Some(word.clone());
        self.hover = Some(HoverCard {
            range: word.clone(),
            diagnostics: Vec::new(),
            blocks: vec![HoverBlock { code: false, text: message }],
        });
        self.hover_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update(cx, |this, cx| {
                if this.hover_word.as_ref() == Some(&word) && !this.mouse_in_card {
                    this.close_hover(cx);
                }
            })
            .ok();
        }));
        cx.notify();
    }

    /// Underlines the word under the mouse while Cmd/Ctrl is held, to show it can be clicked.
    fn update_link(&mut self, cx: &mut Context<Self>) {
        let link = (self.secondary_held && self.lsp.is_some())
            .then(|| self.text_under_mouse())
            .flatten()
            .map(|offset| self.word_at(offset))
            .filter(|word| self.buffer.char_at(word.start).is_some_and(|c| c.is_alphanumeric() || c == '_'));
        if link != self.link_word {
            self.link_word = link;
            cx.notify();
        }
    }

    /// Keeps the hover card in step with the mouse and the Alt key.
    ///
    /// Alt opens a card for the word under the mouse; releasing Alt doesn't close
    /// it, so the mouse can move into the card to scroll or select. It closes once
    /// the mouse has been away from both the word and the card for a moment.
    pub(super) fn update_hover(&mut self, cx: &mut Context<Self>) {
        self.update_link(cx);
        let under = self.text_under_mouse();

        if self.hover.is_some() && !self.hover_from_keyboard {
            if self.mouse_in_card || self.is_on_hovered_word(under) {
                self.hover_close_task = None;
            } else if self.hover_close_task.is_none() {
                self.hover_close_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(HOVER_GRACE).await;
                    this.update(cx, |this, cx| {
                        if !this.mouse_in_card && !this.is_on_hovered_word(this.text_under_mouse()) {
                            this.close_hover(cx);
                        }
                    })
                    .ok();
                }));
            }
        }

        if self.alt_held && !self.hover_suppressed && !self.mouse_in_card {
            match under {
                Some(offset) => {
                    // Switching to another word waits a little, so crossing words on the
                    // way to the card doesn't replace it.
                    let delay = if self.hover.is_some() { HOVER_SWITCH_DELAY } else { HOVER_DELAY };
                    self.hover_from_keyboard = false;
                    self.request_hover(offset, delay, cx);
                }
                // Over blank space: forget a pending switch, keep what's shown.
                None => {
                    if self.hover_word != self.hover.as_ref().map(|h| h.range.clone()) {
                        self.hover_task = None;
                        self.hover_word = self.hover.as_ref().map(|h| h.range.clone());
                    }
                }
            }
        }
    }

    fn is_on_hovered_word(&self, under: Option<usize>) -> bool {
        match (&self.hover, under) {
            (Some(card), Some(offset)) => {
                card.range.start <= offset && offset < card.range.end.max(card.range.start + 1)
            }
            _ => false,
        }
    }

    pub(super) fn close_hover(&mut self, cx: &mut Context<Self>) {
        if self.hover.is_some() || self.hover_word.is_some() {
            self.hover = None;
            self.hover_word = None;
            self.hover_task = None;
            self.hover_close_task = None;
            self.hover_from_keyboard = false;
            self.mouse_in_card = false;
            cx.notify();
        }
    }

    /// Called as the mouse enters or leaves the card.
    pub(super) fn set_mouse_in_card(&mut self, inside: bool, cx: &mut Context<Self>) {
        self.mouse_in_card = inside;
        if inside {
            // Reaching the card cancels a pending switch to a word crossed on the way.
            self.hover_task = None;
            self.hover_word = self.hover.as_ref().map(|h| h.range.clone());
            self.hover_close_task = None;
        } else {
            self.update_hover(cx);
        }
    }

    /// Shows the card for the word at the caret, without the mouse.
    pub(super) fn show_info_at_caret(&mut self, cx: &mut Context<Self>) {
        let offset = self.selection.head;
        let offset = if self.buffer.char_at(offset).is_some_and(|c| c.is_alphanumeric() || c == '_') {
            offset
        } else {
            offset.saturating_sub(1)
        };
        self.hover_word = None;
        self.hover_from_keyboard = true;
        self.request_hover(offset, Duration::ZERO, cx);
    }

    fn request_hover(&mut self, offset: usize, delay: Duration, cx: &mut Context<Self>) {
        let word = self.word_at(offset);
        if self.hover_word.as_ref() == Some(&word) {
            return;
        }
        self.hover_word = Some(word.clone());
        let mut diagnostics: Vec<(DiagnosticSeverity, String)> = self
            .problems(cx)
            .into_iter()
            .filter(|p| p.range.start <= offset && offset <= p.range.end.max(p.range.start + 1))
            .map(|p| (p.severity, p.message))
            .collect();
        let not_ready = self.not_ready_message(cx);
        let request = if not_ready.is_some() {
            None
        } else {
            self.lsp
                .as_ref()
                .zip(self.path.as_ref())
                .map(|(lsp, path)| lsp.read(cx).hover(path, self.lsp_position(offset)))
        };
        if let Some(message) = not_ready {
            diagnostics.push((DiagnosticSeverity::HINT, message));
        }
        self.hover_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let hover = match request {
                Some(request) => request.await,
                None => None,
            };
            this.update(cx, |this, cx| {
                // The mouse may have moved on while the server answered.
                if this.hover_word.as_ref() != Some(&word) {
                    return;
                }
                let blocks = hover.map(|h| hover_blocks(h.contents)).unwrap_or_default();
                if blocks.is_empty() && diagnostics.is_empty() {
                    if this.hover.is_none() {
                        this.hover_word = None;
                    }
                    return;
                }
                this.hover = Some(HoverCard { range: word, diagnostics, blocks });
                this.hover_close_task = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// The char under the mouse, if the mouse is over text (not blank space).
    fn text_under_mouse(&self) -> Option<usize> {
        let position = self.mouse_position?;
        if !self.layout.as_ref()?.text_bounds.contains(&position) {
            return None;
        }
        let offset = self.offset_at(position);
        let (line, column) = self.buffer.point(offset);
        let on_text =
            column < self.buffer.line_len(line) && self.buffer.char_at(offset).is_some_and(|c| !c.is_whitespace());
        on_text.then_some(offset)
    }

    pub fn go_to_definition_at(&mut self, offset: usize, cx: &mut Context<Self>) {
        if let Some(message) = self.not_ready_message(cx) {
            self.show_notice(offset, message, cx);
            return;
        }
        let (Some(lsp), Some(path)) = (&self.lsp, &self.path) else { return };
        let request = lsp.read(cx).definition(path, self.lsp_position(offset));
        self.definition_task = Some(cx.spawn(async move |this, cx| {
            let Some(location) = request.await.into_iter().next() else {
                this.update(cx, |this, cx| this.show_notice(offset, "No definition found here.".into(), cx)).ok();
                return;
            };
            let Some(target) = path_for(&location.uri) else { return };
            this.update(cx, |this, cx| {
                if this.path.as_deref() == Some(target.as_path()) {
                    this.select_lsp_range(location.range, cx);
                } else {
                    cx.emit(EditorEvent::GoTo { path: target, range: location.range });
                }
            })
            .ok();
        }));
    }

    pub fn select_lsp_range(&mut self, range: lsp_types::Range, cx: &mut Context<Self>) {
        let start = self.offset_from_lsp(range.start);
        let end = self.offset_from_lsp(range.end);
        self.selection = Selection { anchor: start, head: end };
        self.goal_column = None;
        self.touch(cx);
    }
}

static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").unwrap());

/// Splits a hover's markdown into code blocks and plain paragraphs.
pub fn hover_blocks(contents: HoverContents) -> Vec<HoverBlock> {
    let marked = |m: MarkedString| match m {
        MarkedString::String(s) => s,
        MarkedString::LanguageString(l) => format!("```{}\n{}\n```", l.language, l.value),
    };
    let markdown = match contents {
        HoverContents::Scalar(m) => marked(m),
        HoverContents::Array(items) => items.into_iter().map(marked).collect::<Vec<_>>().join("\n\n"),
        HoverContents::Markup(m) => m.value,
    };
    let mut blocks = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    let mut in_code = false;
    let flush = |lines: &mut Vec<&str>, code: bool, blocks: &mut Vec<HoverBlock>| {
        let text = lines.join("\n");
        lines.clear();
        let text = if code { text.trim_end().to_string() } else { clean_markdown(&text) };
        if !text.trim().is_empty() {
            blocks.push(HoverBlock { code, text });
        }
    };
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            flush(&mut lines, in_code, &mut blocks);
            in_code = !in_code;
        } else if !in_code && line.trim() == "---" {
            flush(&mut lines, false, &mut blocks);
        } else {
            lines.push(line);
        }
    }
    flush(&mut lines, in_code, &mut blocks);

    // Keep long docs from covering the screen.
    let mut budget = MAX_HOVER_LINES;
    blocks
        .into_iter()
        .filter_map(|mut b| {
            if budget == 0 {
                return None;
            }
            let lines: Vec<&str> = b.text.lines().take(budget).collect();
            budget -= lines.len();
            b.text = lines.join("\n");
            Some(b)
        })
        .collect()
}

/// Turns markdown into readable plain text: no heading marks, links or emphasis.
fn clean_markdown(text: &str) -> String {
    let text = LINK.replace_all(text, "$1").replace("**", "").replace("__", "").replace('`', "");
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let line = if let Some(rest) = trimmed.strip_prefix('#') {
            rest.trim_start_matches('#').trim().to_string()
        } else if let Some(rest) = trimmed.strip_prefix("* ").or_else(|| trimmed.strip_prefix("- ")) {
            format!("• {rest}")
        } else if let Some(rest) = trimmed.strip_prefix('>') {
            rest.trim().to_string()
        } else {
            line.to_string()
        };
        // Collapse runs of blank lines.
        if line.trim().is_empty() && out.last().is_none_or(|l| l.trim().is_empty()) {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{MarkupContent, MarkupKind};

    #[test]
    fn splits_rust_analyzer_style_hover() {
        let value = "```rust\nnull_ide::buffer\n```\n\n```rust\npub struct Buffer\n```\n\n---\n\nText **storage**, see [ropey](https://x).";
        let blocks =
            hover_blocks(HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: value.into() }));
        let texts: Vec<(bool, &str)> = blocks.iter().map(|b| (b.code, b.text.as_str())).collect();
        assert_eq!(
            texts,
            vec![(true, "null_ide::buffer"), (true, "pub struct Buffer"), (false, "Text storage, see ropey.")]
        );
    }

    #[test]
    fn cleans_headings_lists_and_quotes() {
        assert_eq!(clean_markdown("# Title\n\n\n* one\n> note"), "Title\n\n• one\nnote");
    }
}
