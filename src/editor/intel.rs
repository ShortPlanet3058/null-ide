//! Code intelligence for the editor, backed by a language server: diagnostics,
//! hover cards (hold Alt/Option) and go to definition.

use super::{Editor, EditorEvent, Selection};
use crate::lsp::path_for;
use crate::lsp_store::{LspStore, Readiness};
use crate::markdown;
use gpui::{App, Context, Entity};
use lsp_types::{DiagnosticSeverity, HoverContents, MarkedString, Position, TextDocumentContentChangeEvent};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

/// Alt over a word this long shows its card.
const HOVER_DELAY: Duration = Duration::from_millis(80);
/// With a card open, the mouse has to rest on another word this long to switch to it.
const HOVER_SWITCH_DELAY: Duration = Duration::from_millis(350);
/// How long the mouse can be away from the word and the card before the card closes.
const HOVER_GRACE: Duration = Duration::from_millis(450);
/// Cards show the signature and the start of the docs, not whole READMEs.
const MAX_HOVER_LINES: usize = 14;
/// The lines of `text`, ended by \n, \r\n or a lone \r (as language servers count them).
fn text_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    while let Some(end) = rest.find(['\n', '\r']) {
        lines.push(&rest[..end]);
        let skip = if rest[end..].starts_with("\r\n") { 2 } else { 1 };
        rest = &rest[end + skip..];
    }
    if !rest.is_empty() {
        lines.push(rest);
    }
    lines
}

/// Peek Definition shows this many lines from the definition on.
const PEEK_LINES: usize = 14;

/// The lines of `text` from `line` on that Peek Definition shows: the definition, up to
/// `PEEK_LINES` and until a line less indented than its first (the end of what holds it),
/// that indentation taken off, and no blank lines at the end.
fn peek_lines(text: &str, line: usize) -> String {
    let indent = |l: &str| l.len() - l.trim_start().len();
    // Lines as language servers count them: a lone \r ends one too.
    let mut lines = text_lines(text).into_iter().skip(line).map(str::trim_end);
    let Some(first) = lines.next() else { return String::new() };
    let base = indent(first);
    // Deeper lines are its body; one as deep is its end when it closes (`}`, `)`, `end`),
    // the next definition otherwise.
    let mut shown: Vec<&str> = vec![first];
    for l in lines.take(PEEK_LINES - 1) {
        if l.is_empty() || indent(l) > base {
            shown.push(l);
            continue;
        }
        if indent(l) == base && (l.trim_start().starts_with(['}', ')', ']']) || l.trim() == "end") {
            shown.push(l);
        }
        break;
    }
    while shown.last().is_some_and(|l| l.is_empty()) {
        shown.pop();
    }
    shown.iter().map(|l| l.get(base..).unwrap_or("")).collect::<Vec<_>>().join("\n")
}

pub struct HoverCard {
    pub range: Range<usize>,
    pub diagnostics: Vec<(DiagnosticSeverity, String)>,
    pub blocks: Vec<HoverBlock>,
    /// An image whose path is written here (`![](shot.png)`, `src="logo.svg"`), shown, and
    /// what's said under it (its name and size), read once.
    pub image: Option<(std::path::PathBuf, String)>,
}

pub struct HoverBlock {
    pub code: bool,
    pub text: String,
}

/// A problem reported by the language server, in char offsets.
#[derive(Clone)]
pub struct Problem {
    pub range: Range<usize>,
    pub severity: DiagnosticSeverity,
    pub message: String,
    /// As the server sent it, to hand back when asking for fixes.
    pub diagnostic: lsp_types::Diagnostic,
}

/// The server's problems for this file, each pinned to its text (byte ranges) so it
/// moves with edits until the server says otherwise. None for a problem whose text
/// was deleted: it stays gone even when the server repeats it unchanged.
#[derive(Default)]
pub(super) struct Pinned {
    version: u64,
    revision: u64,
    items: Vec<(lsp_types::Diagnostic, Option<Range<usize>>)>,
}

/// Where a byte range ends up after `edit`, or None once the text it covered is replaced.
pub(super) fn map_range(range: Range<usize>, edit: &crate::buffer::Edit) -> Option<Range<usize>> {
    let (start, old_end, new_end) = (edit.start_byte, edit.old_end_byte, edit.new_end_byte);
    if start < old_end && start <= range.start && range.end <= old_end {
        return None;
    }
    let shift = |p: usize| p - old_end + new_end;
    // Typing right before a problem pushes it along; typing right after leaves it be.
    let first = if range.start < start {
        range.start
    } else if range.start >= old_end {
        shift(range.start)
    } else {
        start
    };
    let last = if range.end <= start {
        range.end
    } else if range.end >= old_end {
        shift(range.end)
    } else {
        new_end
    };
    Some(first..last.max(first))
}

impl Editor {
    pub(super) fn attach_lsp(&mut self, lsp: Entity<LspStore>, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return };
        if !lsp.read(cx).has_server_for(&path) {
            return;
        }
        let text = self.buffer.rope().clone();
        self.lsp_revision = self.buffer.revision();
        // A second copy of a file open twice leaves the talking to the first.
        if !self.lsp_follower {
            lsp.update(cx, |lsp, cx| lsp.open(&path, text, cx));
        }
        self.lsp_subscription = Some(cx.observe(&lsp, |_, _, cx| cx.notify()));
        self.lsp = Some(lsp);
    }

    /// Tells the server the file is no longer open. Call before dropping the editor.
    pub fn release_lsp(&mut self, cx: &mut Context<Self>) {
        if let (Some(lsp), Some(path)) = (self.lsp.take().filter(|_| !self.lsp_follower), self.path.clone()) {
            lsp.update(cx, |lsp, cx| lsp.close(&path, cx));
        }
        self.lsp_subscription = None;
    }

    /// The other copy of this file closed: this one tells the server about the file now.
    pub fn lead_lsp(&mut self, cx: &mut Context<Self>) {
        if !self.lsp_follower {
            return;
        }
        self.lsp_follower = false;
        if let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) {
            self.lsp_revision = self.buffer.revision();
            let text = self.buffer.rope().clone();
            lsp.update(cx, |lsp, cx| lsp.open(&path, text, cx));
        }
    }

    pub(super) fn sync_lsp(&mut self, cx: &mut Context<Self>) {
        if self.lsp_follower {
            return;
        }
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else { return };
        self.lsp_version += 1;
        // The edits since the server last heard, as it counts positions; a few bytes per keystroke.
        // Any edit it can't be told as a range (one splitting a "\r\n"): the whole text instead.
        let edits = self.buffer.edits_since(self.lsp_revision).and_then(|edits| {
            edits
                .map(|e| {
                    let ((start_line, start_col), (end_line, end_col)) = e.lsp_range?;
                    Some(TextDocumentContentChangeEvent {
                        range: Some(lsp_types::Range {
                            start: Position { line: start_line, character: start_col },
                            end: Position { line: end_line, character: end_col },
                        }),
                        range_length: None,
                        text: e.text.clone(),
                    })
                })
                .collect::<Option<Vec<_>>>()
        });
        self.lsp_revision = self.buffer.revision();
        let (text, version) = (self.buffer.rope().clone(), self.lsp_version);
        lsp.update(cx, |lsp, cx| lsp.change(&path, text, edits, version, cx));
    }

    pub(super) fn lsp_saved(&mut self, cx: &mut Context<Self>) {
        if let (Some(lsp), Some(path)) = (self.lsp.clone().filter(|_| !self.lsp_follower), self.path.clone()) {
            lsp.update(cx, |lsp, cx| lsp.save(&path, cx));
        }
    }

    pub(super) fn lsp_position(&self, offset: usize) -> Position {
        let (line, column) = self.buffer.point(offset);
        Position { line: line as u32, character: self.buffer.column_to_utf16(line, column) as u32 }
    }

    pub fn offset_from_lsp(&self, position: Position) -> usize {
        let line = position.line as usize;
        self.buffer.offset(line, self.buffer.utf16_to_column(line, position.character as usize))
    }

    /// Errors, warnings and notes the server reported for this file.
    /// The server's problems for this file, in the current text. Drawn every frame, so
    /// worked out once per change of the text or of the diagnostics.
    pub fn problems(&self, cx: &App) -> Rc<Vec<Problem>> {
        let (Some(lsp), Some(path)) = (&self.lsp, &self.path) else { return Rc::default() };
        let lsp = lsp.read(cx);
        let key = (lsp.diagnostics_version(), self.buffer.revision());
        if let Some((k, problems)) = &*self.problems_cache.borrow()
            && *k == key
        {
            return problems.clone();
        }
        let (version, revision) = key;
        let mut pinned = self.pinned.borrow_mut();
        // Bring the pinned problems up to the current text.
        let mut lost_track = false;
        if pinned.revision != revision {
            match self.buffer.edits_since(pinned.revision) {
                Some(edits) => {
                    let edits: Vec<_> = edits.collect();
                    for (_, range) in &mut pinned.items {
                        *range = range.take().and_then(|r| edits.iter().try_fold(r, |r, e| map_range(r, e)));
                    }
                }
                // After an undo (or too many edits to follow), go by the server's positions.
                None => lost_track = true,
            }
            pinned.revision = revision;
        }
        // New problems from the server: ones it repeats unchanged (rust-analyzer resends
        // cargo check's until the next save) keep their pinned place.
        if pinned.version != version || lost_track {
            let rope = self.buffer.rope();
            let byte = |p: Position| rope.char_to_byte(self.offset_from_lsp(p).min(rope.len_chars()));
            let mut previous = std::mem::take(&mut pinned.items);
            pinned.items = lsp
                .diagnostics(path)
                .iter()
                .map(|d| {
                    let kept = (!lost_track)
                        .then(|| previous.iter().position(|(old, _)| old == d))
                        .flatten()
                        .map(|i| previous.swap_remove(i).1);
                    (d.clone(), kept.unwrap_or_else(|| Some(byte(d.range.start)..byte(d.range.end))))
                })
                .collect();
            pinned.version = version;
        }
        let rope = self.buffer.rope();
        let char_at = |b: usize| rope.byte_to_char(b.min(rope.len_bytes()));
        let problems: Rc<Vec<Problem>> = Rc::new(
            pinned
                .items
                .iter()
                .filter_map(|(d, range)| {
                    let range = range.clone()?;
                    Some(Problem {
                        range: char_at(range.start)..char_at(range.end),
                        severity: d.severity.unwrap_or(DiagnosticSeverity::ERROR),
                        message: d.message.clone(),
                        diagnostic: d.clone(),
                    })
                })
                .collect(),
        );
        *self.problems_cache.borrow_mut() = Some((key, problems.clone()));
        problems
    }

    /// This file's problems as a server would describe them for the text as it is now;
    /// None without a language server, which knows where they are as well as this does.
    pub fn current_diagnostics(&self, cx: &App) -> Option<Vec<lsp_types::Diagnostic>> {
        self.lsp.as_ref()?;
        let found = self
            .problems(cx)
            .iter()
            .map(|p| lsp_types::Diagnostic {
                range: lsp_types::Range {
                    start: self.lsp_position(p.range.start),
                    end: self.lsp_position(p.range.end),
                },
                ..p.diagnostic.clone()
            })
            .collect();
        Some(found)
    }

    /// Shows, moves or hides the hover card to match the mouse and the Alt key.
    pub fn readiness(&self, cx: &App) -> Option<Readiness> {
        self.lsp.as_ref()?.read(cx).readiness(self.path.as_ref()?)
    }

    /// Why hover and go to definition can't answer yet, in words.
    pub(super) fn not_ready_message(&self, cx: &App) -> Option<String> {
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
            Readiness::Missing { server } => {
                Some(format!("{label} needs {}: install it from the status bar to get info here.", server.name))
            }
            Readiness::Installing { server } => Some(format!("Installing {}…", server.name)),
            Readiness::InstallFailed { server } => {
                Some(format!("Installing {} failed: see the status bar.", server.name))
            }
            Readiness::Unavailable { program } => Some(format!("{program} didn't start.")),
        }
    }

    /// Shows a short message in the hover card spot for a few seconds.
    pub(super) fn show_notice(&mut self, offset: usize, message: String, cx: &mut Context<Self>) {
        let word = self.word_at(offset);
        self.hover_word = Some(word.clone());
        self.hover = Some(HoverCard {
            range: word.clone(),
            diagnostics: Vec::new(),
            blocks: vec![HoverBlock { code: false, text: message }],
            image: None,
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

    /// Underlines what's under the mouse while Cmd/Ctrl is held, to show it can be clicked:
    /// a web address or a file's path, or a name to go to the definition of.
    fn update_link(&mut self, cx: &mut Context<Self>) {
        let offset = self.secondary_held.then(|| self.text_under_mouse()).flatten();
        let link = offset.and_then(|offset| self.link_under(offset).map(|(range, _)| range)).or_else(|| {
            offset
                .filter(|_| self.lsp.is_some())
                .map(|offset| self.word_at(offset))
                .filter(|word| self.buffer.char_at(word.start).is_some_and(|c| c.is_alphanumeric() || c == '_'))
        });
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
        if self.request_image_hover(offset, delay, cx) {
            return;
        }
        let word = self.word_at(offset);
        if self.hover_word.as_ref() == Some(&word) {
            return;
        }
        self.hover_word = Some(word.clone());
        let mut diagnostics: Vec<(DiagnosticSeverity, String)> = self
            .problems(cx)
            .iter()
            .filter(|p| p.range.start <= offset && offset <= p.range.end.max(p.range.start + 1))
            .map(|p| (p.severity, p.message.clone()))
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
        // While the debugger is stopped here: the name's value, first.
        let name = self.buffer.slice(word.clone());
        let value = self
            .debug_locals
            .iter()
            .find(|(local, _)| *local == name)
            .map(|(local, value)| HoverBlock { code: true, text: format!("{local} = {value}") });
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
                let mut blocks = hover.map(|h| hover_blocks(h.contents)).unwrap_or_default();
                if let Some(value) = value {
                    blocks.insert(0, value);
                }
                if blocks.is_empty() && diagnostics.is_empty() {
                    if this.hover.is_none() {
                        this.hover_word = None;
                    }
                    return;
                }
                this.hover = Some(HoverCard { range: word, diagnostics, blocks, image: None });
                this.hover_close_task = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Over the path of an image that exists: the card shows the image. True when it is one.
    fn request_image_hover(&mut self, offset: usize, delay: Duration, cx: &mut Context<Self>) -> bool {
        let Some((range, super::links::Target::File { path, .. })) = self.link_under(offset) else { return false };
        if !crate::preview::is_image(&path) {
            return false;
        }
        if self.hover_word.as_ref() == Some(&range) {
            return true;
        }
        self.hover_word = Some(range.clone());
        self.hover_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                if this.hover_word.as_ref() != Some(&range) {
                    return;
                }
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let about = crate::preview::of(&path, &Err(std::io::ErrorKind::InvalidData))
                    .filter(|p| matches!(p, crate::preview::Preview::Image { .. }))
                    .map_or(name.clone(), |p| format!("{name} · {}", p.summary()));
                this.hover =
                    Some(HoverCard { range, diagnostics: Vec::new(), blocks: Vec::new(), image: Some((path, about)) });
                this.hover_close_task = None;
                cx.notify();
            })
            .ok();
        }));
        true
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

    /// Where the symbol at the caret's type is defined, or where it's implemented: one
    /// place is gone to, several are listed.
    pub(super) fn go_to_target(&mut self, target: crate::lsp_store::Target, cx: &mut Context<Self>) {
        use crate::lsp_store::Target;
        let offset = self.selection.head;
        if let Some(message) = self.not_ready_message(cx) {
            return self.show_notice(offset, message, cx);
        }
        let (Some(lsp), Some(path)) = (&self.lsp, &self.path) else { return };
        let request = lsp.read(cx).locations_of(target, path, self.lsp_position(offset));
        let word = self.buffer.slice(self.word_at(offset));
        self.definition_task = Some(cx.spawn(async move |this, cx| {
            let found = request.await;
            this.update(cx, |this, cx| {
                let (none, many) = if target == Target::Implementation {
                    ("No implementation found here.", "implementations")
                } else {
                    ("No type definition found here.", "types")
                };
                this.go_or_list(found, offset, none, &format!("{many} of {word}"), cx);
            })
            .ok();
        }));
    }

    /// ⌃⌥H: where the function at the caret is called from, listed (or gone to, when once).
    pub(super) fn show_callers(&mut self, _: &super::ShowCallers, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let offset = self.selection.head;
        if let Some(message) = self.not_ready_message(cx) {
            return self.show_notice(offset, message, cx);
        }
        let (Some(lsp), Some(path)) = (&self.lsp, &self.path) else { return };
        let request = lsp.read(cx).callers(path, self.lsp_position(offset));
        let word = self.buffer.slice(self.word_at(offset));
        self.definition_task = Some(cx.spawn(async move |this, cx| {
            let found = request.await;
            this.update(cx, |this, cx| {
                this.go_or_list(found, offset, "Nothing calls this here.", &format!("callers of {word}"), cx)
            })
            .ok();
        }));
    }

    /// Places found for the symbol at `offset`: none says so, one is gone to, several are
    /// listed as "{count} {what}".
    fn go_or_list(
        &mut self,
        found: Vec<lsp_types::Location>,
        offset: usize,
        none: &str,
        what: &str,
        cx: &mut Context<Self>,
    ) {
        match found.as_slice() {
            [] => self.show_notice(offset, none.into(), cx),
            [location] => {
                let Some(target) = path_for(&location.uri) else { return };
                if self.path.as_deref() == Some(target.as_path()) {
                    cx.emit(EditorEvent::Jumped { from: self.caret_point() });
                    self.select_lsp_range(location.range, cx);
                } else {
                    cx.emit(EditorEvent::GoTo { path: target, range: location.range });
                }
            }
            many => cx
                .emit(EditorEvent::ShowLocations { title: format!("{} {what}", many.len()), locations: many.to_vec() }),
        }
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
                let here = this.path.as_deref() == Some(target.as_path());
                // Already at the definition: show where it's used instead, as other editors do.
                let at_definition = here && {
                    let range = this.offset_from_lsp(location.range.start)..this.offset_from_lsp(location.range.end);
                    range.contains(&offset) || range.end == offset
                };
                if at_definition {
                    this.find_references_at(offset, cx);
                } else if here {
                    cx.emit(EditorEvent::Jumped { from: this.caret_point() });
                    this.select_lsp_range(location.range, cx);
                } else {
                    cx.emit(EditorEvent::GoTo { path: target, range: location.range });
                }
            })
            .ok();
        }));
    }

    /// The definition of the name at `offset`, shown in the info card where the caret is:
    /// its file and line, then its first lines. Nothing moves.
    /// Expand Macro (Rust): what the macro at `offset` turns into, in the card.
    pub fn expand_macro_at(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.language_name() != "Rust" {
            return self.show_notice(offset, "Expand Macro is for Rust (rust-analyzer).".into(), cx);
        }
        if let Some(message) = self.not_ready_message(cx) {
            return self.show_notice(offset, message, cx);
        }
        let (Some(lsp), Some(path)) = (&self.lsp, &self.path) else { return };
        let request = lsp.read(cx).expand_macro(path, self.lsp_position(offset));
        self.definition_task = Some(cx.spawn(async move |this, cx| {
            let expanded = request.await;
            this.update(cx, |this, cx| {
                let Some(expanded) = expanded.filter(|e| !e.expansion.trim().is_empty()) else {
                    return this.show_notice(offset, "No macro to expand here.".into(), cx);
                };
                let word = this.word_at(offset);
                this.hover_word = Some(word.clone());
                this.hover_from_keyboard = true;
                this.hover = Some(HoverCard {
                    range: word,
                    diagnostics: Vec::new(),
                    blocks: vec![
                        HoverBlock {
                            code: false,
                            text: format!("{}! expands to", expanded.name.trim_end_matches('!')),
                        },
                        HoverBlock { code: true, text: expanded.expansion.trim_end().to_string() },
                    ],
                    image: None,
                });
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn peek_definition_at(&mut self, offset: usize, cx: &mut Context<Self>) {
        if let Some(message) = self.not_ready_message(cx) {
            self.show_notice(offset, message, cx);
            return;
        }
        let (Some(lsp), Some(path)) = (&self.lsp, &self.path) else {
            return self.show_notice(offset, "Definitions come from a language server, and none runs here.".into(), cx);
        };
        let request = lsp.read(cx).definition(path, self.lsp_position(offset));
        self.definition_task = Some(cx.spawn(async move |this, cx| {
            let Some(location) = request.await.into_iter().next() else {
                this.update(cx, |this, cx| this.show_notice(offset, "No definition found here.".into(), cx)).ok();
                return;
            };
            let Some(target) = path_for(&location.uri) else { return };
            this.update(cx, |this, cx| {
                let line = location.range.start.line as usize;
                let text = if this.path.as_deref() == Some(target.as_path()) {
                    Some(this.buffer.to_string())
                } else {
                    crate::encoding::read(&target).ok().map(|(text, _)| text)
                };
                let Some(text) = text else { return this.show_notice(offset, "Couldn't read it.".into(), cx) };
                let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let word = this.word_at(offset);
                this.hover_word = Some(word.clone());
                this.hover_from_keyboard = true;
                this.hover = Some(HoverCard {
                    range: word,
                    diagnostics: Vec::new(),
                    blocks: vec![
                        HoverBlock { code: false, text: format!("{name}:{}", line + 1) },
                        HoverBlock { code: true, text: peek_lines(&text, line) },
                    ],
                    image: None,
                });
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn select_lsp_range(&mut self, range: lsp_types::Range, cx: &mut Context<Self>) {
        let start = self.offset_from_lsp(range.start);
        let end = self.offset_from_lsp(range.end);
        self.single_cursor();
        self.selection = Selection { anchor: start, head: end };
        self.goal_column = None;
        self.touch(cx);
    }

    /// Like [`Self::select_lsp_range`], for a list floating over the top of the editor:
    /// the line glides to two thirds down the view, clear of the list.
    pub fn preview_lsp_range(&mut self, range: lsp_types::Range, cx: &mut Context<Self>) {
        self.select_lsp_range(range, cx);
        let Some(layout) = &self.layout else { return };
        let lh = f32::from(layout.line_height);
        let view = f32::from(layout.text_bounds.size.height);
        let line = self.buffer.point(self.selection.head).0;
        self.wrap.update(&self.buffer, self.wrap.width(), &self.block_specs());
        let row = self.wrap.first_row(line) as f32;
        self.scroll.target_y = (row * lh - view * 2. / 3.).max(0.);
        self.autoscroll = false;
        cx.notify();
    }
}

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
    markdown::blocks(&markdown, Some(MAX_HOVER_LINES))
        .into_iter()
        .map(|b| HoverBlock { code: b.code, text: b.text })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{MarkupContent, MarkupKind};

    fn insert(at: usize, len: usize) -> crate::buffer::Edit {
        edit(at, at, at + len)
    }

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
    fn problems_move_with_the_text() {
        // Typed before: pushed along. Typed right after: left be. Typed inside: grows.
        assert_eq!(map_range(10..15, &insert(2, 3)), Some(13..18));
        assert_eq!(map_range(10..15, &insert(10, 3)), Some(13..18));
        assert_eq!(map_range(10..15, &insert(15, 3)), Some(10..15));
        assert_eq!(map_range(10..15, &insert(12, 3)), Some(10..18));
        // Text before it deleted: pulled back. Its own text replaced: gone.
        assert_eq!(map_range(10..15, &edit(0, 4, 0)), Some(6..11));
        assert_eq!(map_range(10..15, &edit(8, 16, 9)), None);
        // Part of it deleted: what's left.
        assert_eq!(map_range(10..15, &edit(12, 20, 12)), Some(10..12));
    }

    #[test]
    fn peeks_at_the_definition_s_first_lines() {
        let text = "mod a {\n    /// Doc.\n    pub fn f(x: u32) -> u32 {\n        x + 1\n    }\n\n\n}\n";
        // The function, not the module's closing brace after it.
        assert_eq!(peek_lines(text, 2), "pub fn f(x: u32) -> u32 {\n    x + 1\n}");
        // No more than PEEK_LINES, no blank lines left at the end.
        let long: String =
            "fn long() {\n".to_string() + &(0..40).map(|i| format!("    line {i}\n")).collect::<String>();
        assert_eq!(peek_lines(&long, 0).lines().count(), PEEK_LINES);
        assert_eq!(peek_lines("a\n\n\n", 0), "a");
        // At the top level: the function and its closing brace, not what follows; a
        // one-line definition alone; a lone \r ends a line.
        assert_eq!(peek_lines("fn f() {\n    1\n}\n\nfn g() {}\n", 0), "fn f() {\n    1\n}");
        assert_eq!(peek_lines("const X: u32 = 1;\nconst Y: u32 = 2;\n", 0), "const X: u32 = 1;");
        assert_eq!(peek_lines("a\rdef f():\r    pass\rb = 1\r", 1), "def f():\n    pass");
    }

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

    #[gpui::test]
    fn while_paused_the_info_card_starts_with_the_value(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(Buffer::from_text("total += word.len();\n"), Some(std::path::PathBuf::from("x.txt")), cx)
        });
        e.update(cx, |e, cx| {
            e.debug_locals = vec![("word".into(), "\"null\"".into()), ("total".into(), "4".into())];
            e.request_hover(10, Duration::ZERO, cx);
        });
        cx.run_until_parked();
        e.update(cx, |e, _| {
            let card = e.hover.as_ref().expect("a card");
            assert_eq!(card.blocks[0].text, "word = \"null\"");
        });
    }

    /// ⌥ over an image's path (Markdown, HTML, an import) shows the image; over a word that
    /// isn't one, the usual card.
    #[gpui::test]
    fn an_image_s_path_shows_the_image(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let dir = crate::tools::test_dir("image-hover");
        std::fs::create_dir_all(dir.join("img")).unwrap();
        for f in ["shot.png", "img/logo.svg", "img/hero.jpg"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let text = "See ![a shot](shot.png) and <img src=\"img/logo.svg\">.\nimport hero from './img/hero.jpg';\nmissing.png\n";
        let file = dir.join("notes.md");
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(file), cx));
        let shown = |at: &str, cx: &mut gpui::VisualTestContext| {
            let offset = text[..text.find(at).unwrap()].chars().count() + 1;
            e.update(cx, |e, cx| {
                e.close_hover(cx);
                e.request_hover(offset, Duration::ZERO, cx);
            });
            cx.run_until_parked();
            e.read_with(cx, |e, _| {
                e.hover
                    .as_ref()
                    .and_then(|h| h.image.as_ref())
                    .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            })
        };
        assert_eq!(shown("shot.png", cx).as_deref(), Some("shot.png"));
        assert_eq!(shown("img/logo", cx).as_deref(), Some("logo.svg"));
        assert_eq!(shown("./img/hero", cx).as_deref(), Some("hero.jpg"));
        assert_eq!(shown("missing", cx), None, "no such file");
        assert_eq!(shown("See", cx), None, "not a path");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// With the mouse while debugging: ⌥ held over a name shows its value.
    #[gpui::test]
    fn holding_alt_over_a_name_shows_its_value(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(Buffer::from_text("total += word.len();\n"), Some(std::path::PathBuf::from("x.txt")), cx)
        });
        e.update_in(cx, |e, window, _| {
            e.debug_locals = vec![("word".into(), "\"null\"".into())];
            window.focus(&e.focus_handle);
        });
        cx.run_until_parked();
        // Over "word" (columns 9 to 13).
        let over = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().expect("drawn");
            gpui::point(l.text_origin.x + l.char_width * 10.5, l.text_origin.y + l.line_height * 0.5)
        });
        let alt = gpui::Modifiers { alt: true, ..Default::default() };
        cx.simulate_mouse_move(over, None, alt);
        cx.simulate_modifiers_change(alt);
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        e.read_with(cx, |e, _| {
            let card = e.hover.as_ref().expect("a card while ⌥ is held");
            assert_eq!(card.blocks[0].text, "word = \"null\"");
        });
    }

    #[gpui::test]
    fn problems_stay_on_their_text_until_the_server_says_otherwise(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        use gpui::AppContext as _;
        use lsp_types::{Diagnostic, Range as LspRange};
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let path = std::path::PathBuf::from("/tmp/x.rs");
        let lsp = cx.new(|_| LspStore::new("/tmp".into()));
        let (e, cx) = cx
            .add_window_view(|_, cx| Editor::new(Buffer::from_text("fn main() {\n    x\n}\n"), Some(path.clone()), cx));
        let stale = Diagnostic {
            range: LspRange { start: Position::new(1, 4), end: Position::new(1, 5) },
            message: "cannot find value `x`".into(),
            ..Default::default()
        };
        let publish = |list: Vec<Diagnostic>, cx: &mut gpui::VisualTestContext| {
            lsp.update(cx, |lsp, _| lsp.set_diagnostics(path.clone(), list));
        };
        publish(vec![stale.clone()], cx);
        e.update(cx, |e, cx| {
            e.lsp = Some(lsp.clone());
            let x = e.buffer.offset(1, 4);
            assert_eq!(e.problems(cx)[0].range, x..x + 1);
            // A line added above: the problem goes down with `x`.
            e.buffer.replace(0..0, "// note\n");
            assert_eq!(e.problems(cx)[0].range, x + 8..x + 9);
        });
        // The server repeats it unchanged: it stays on `x`.
        publish(vec![stale.clone()], cx);
        e.update(cx, |e, cx| {
            let x = e.buffer.offset(2, 4);
            assert_eq!(e.problems(cx)[0].range, x..x + 1);
            // `x` deleted: the problem goes, and a repeat doesn't bring it back.
            e.buffer.replace(x..x + 1, "");
            assert!(e.problems(cx).is_empty());
        });
        publish(vec![stale], cx);
        e.update(cx, |e, cx| assert!(e.problems(cx).is_empty()));
    }
}
