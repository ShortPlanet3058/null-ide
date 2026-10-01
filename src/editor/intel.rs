//! Code intelligence for the editor, backed by a language server: diagnostics,
//! hover cards (hold Alt/Option) and go to definition.

use super::{Editor, EditorEvent, Selection};
use crate::lsp::path_for;
use crate::lsp_store::LspStore;
use gpui::{App, Context, Entity};
use lsp_types::{DiagnosticSeverity, HoverContents, MarkedString, Position};
use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;
use std::time::Duration;

/// Holding Alt over a word this long shows its card.
const HOVER_DELAY: Duration = Duration::from_millis(80);
const MAX_HOVER_LINES: usize = 30;

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
    pub(super) fn update_hover(&mut self, cx: &mut Context<Self>) {
        let target = self.hover_target();
        let Some(offset) = target else {
            if self.hover.is_some() || self.hover_word.is_some() {
                self.hover = None;
                self.hover_word = None;
                self.hover_task = None;
                cx.notify();
            }
            return;
        };
        let word = self.word_at(offset);
        if self.hover_word.as_ref() == Some(&word) {
            return;
        }
        self.hover_word = Some(word.clone());
        let diagnostics: Vec<(DiagnosticSeverity, String)> = self
            .problems(cx)
            .into_iter()
            .filter(|p| p.range.start <= offset && offset <= p.range.end.max(p.range.start + 1))
            .map(|p| (p.severity, p.message))
            .collect();
        let request = self
            .lsp
            .as_ref()
            .zip(self.path.as_ref())
            .map(|(lsp, path)| lsp.read(cx).hover(path, self.lsp_position(offset)));
        self.hover_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_DELAY).await;
            let hover = match request {
                Some(request) => request.await,
                None => None,
            };
            this.update(cx, |this, cx| {
                let blocks = hover.map(|h| hover_blocks(h.contents)).unwrap_or_default();
                this.hover = (!blocks.is_empty() || !diagnostics.is_empty()).then_some(HoverCard {
                    range: word,
                    diagnostics,
                    blocks,
                });
                cx.notify();
            })
            .ok();
        }));
    }

    /// The char under the mouse, while Alt is held and the mouse is over text.
    fn hover_target(&self) -> Option<usize> {
        if !self.alt_held || self.hover_suppressed {
            return None;
        }
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
        let (Some(lsp), Some(path)) = (&self.lsp, &self.path) else { return };
        let request = lsp.read(cx).definition(path, self.lsp_position(offset));
        self.definition_task = Some(cx.spawn(async move |this, cx| {
            let Some(location) = request.await.into_iter().next() else { return };
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
        let text = if code { text.trim_end().to_string() } else { clean_markdown(text.trim()) };
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

fn clean_markdown(text: &str) -> String {
    LINK.replace_all(text, "$1").replace("**", "").replace("__", "").replace('`', "")
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
}
