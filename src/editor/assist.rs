//! AI at the code (⌘I). A one-line field opens between the lines; a change is written
//! straight into the file and shown as a diff (removed lines struck through, new ones
//! tinted) to keep with ⇥ or undo with Esc; a question gets a note under the code.
//! Nothing else: no panel, no card over the code.

use super::{EditKind, Editor, EditorEvent, Selection};
use crate::ai::{self, AiEvent, Prompt};
use crate::fonts::CodeFont;
use crate::settings::Settings;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use crate::ui;
use crate::wrap::BlockSpec;
use futures::StreamExt;
use gpui::{
    AnyElement, App, Context, Entity, Focusable, FontWeight, KeyBinding, ScrollHandle, Subscription, Task, Window,
    actions, div, prelude::*, px,
};
use lsp_types::DiagnosticSeverity;
use std::ops::Range;

actions!(inline_ai, [SubmitPrompt, CancelPrompt, KeepChange, UndoChange, CloseNote]);

/// Registered after the editor's own keys, so ⇥ and Esc mean keep and undo while a change is shown.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", SubmitPrompt, Some("AiPrompt")),
        KeyBinding::new("escape", CancelPrompt, Some("AiPrompt")),
        // While it writes the keys are the editor's (its field is gone): Esc stops it.
        KeyBinding::new("escape", CancelPrompt, Some("Editor && ai_writing")),
        KeyBinding::new("tab", KeepChange, Some("Editor && ai_change")),
        KeyBinding::new("escape", UndoChange, Some("Editor && ai_change")),
        KeyBinding::new("escape", CloseNote, Some("Editor && ai_note")),
    ]);
}

/// Items a ⌘I without a selection works on, smallest first.
const RUST_SCOPES: &[&str] =
    &["function_item", "struct_item", "enum_item", "impl_item", "trait_item", "macro_definition", "mod_item"];
/// Whole files above this many lines are trimmed to the lines around the target.
const CONTEXT_LINES: usize = 600;
const NOTE_MAX_ROWS: usize = 14;

/// Rows between the lines of code, drawn by the editor or with something on top.
pub enum BlockKind {
    /// The one-line field.
    Prompt,
    /// Lines a change removed, shown struck through until it's kept or undone.
    Removed(Vec<String>),
    /// The keys for keeping or undoing a change, under it.
    Hint,
    /// The answer to a question.
    Note,
    /// A ghost completion's lines below the caret's line.
    Ghost(Vec<String>),
    /// The new code as the AI writes it, above the code it replaces.
    Writing(Vec<String>),
    /// The keys for a reviewed change, with how many more there are.
    ReviewHint(usize),
}

pub struct Block {
    pub before_line: usize,
    pub rows: usize,
    pub kind: BlockKind,
}

pub(super) struct Prompting {
    input: Entity<TextInput>,
    lines: Range<usize>,
    original: String,
    /// An error on those lines: Enter with nothing typed fixes it.
    error: Option<String>,
    /// Opened to ask about the code, not to change it.
    ask_only: bool,
    writing: bool,
    /// What the AI has written so far, shown as it arrives.
    preview: String,
    failed: Option<String>,
    task: Option<Task<()>>,
    _subscription: Subscription,
}

/// A change written into the file, until it's kept or undone.
pub(super) struct Change {
    version: u64,
    pub(super) added: Vec<Range<usize>>,
    removed: Vec<(usize, Vec<String>)>,
    hint_line: usize,
    lines: Range<usize>,
    instruction: String,
}

pub(super) struct Note {
    /// The note sits just above this line, under the code it's about.
    line: usize,
    question: String,
    answer: String,
    done: bool,
    failed: Option<String>,
    scroll: ScrollHandle,
    /// The file's line count when last placed, to follow edits above it.
    lines_total: usize,
    _task: Option<Task<()>>,
}

/// Questions get an answer in a note; anything else is a change to make.
fn is_question(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower.ends_with('?')
        || ["why ", "what ", "how ", "where ", "when ", "which ", "who ", "explain", "is ", "does ", "can ", "should "]
            .iter()
            .any(|w| lower.starts_with(w))
}

pub fn trim_context(text: &str, lines: &Range<usize>) -> String {
    let all: Vec<&str> = text.lines().collect();
    if all.len() <= CONTEXT_LINES {
        return text.to_string();
    }
    let margin = CONTEXT_LINES / 2;
    let start = lines.start.saturating_sub(margin);
    let end = (lines.end + margin).min(all.len());
    format!("[… lines 1-{start} omitted …]\n{}\n[… rest of the file omitted …]", all[start..end].join("\n"))
}

impl Editor {
    /// The lines ⌘I changes: the selected lines, else the Rust item around the
    /// caret (with its doc comments and attributes), else the paragraph around it.
    fn assist_lines(&self) -> Range<usize> {
        if let Some(lines) = self.selected_line_span() {
            return lines;
        }
        let (caret_line, _) = self.caret_point();
        if self.language_name() == "Rust"
            && let Some(lines) = self.rust_item_lines(caret_line)
        {
            return lines;
        }
        let blank = |l: usize| self.buffer.line_text(l).trim().is_empty();
        let mut start = caret_line;
        while start > 0 && !blank(start - 1) {
            start -= 1;
        }
        let mut end = caret_line + 1;
        while end < self.buffer.len_lines() && !blank(end) {
            end += 1;
        }
        start..end
    }

    fn selected_line_span(&self) -> Option<Range<usize>> {
        let selection = self.selection.range();
        if selection.is_empty() {
            return None;
        }
        let (start, _) = self.buffer.point(selection.start);
        let (end_line, end_col) = self.buffer.point(selection.end);
        let end = if end_col == 0 && end_line > start { end_line } else { end_line + 1 };
        Some(start..end)
    }

    fn rust_item_lines(&self, line: usize) -> Option<Range<usize>> {
        let text = self.buffer.to_string();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).ok()?;
        let tree = parser.parse(&text, None)?;
        let byte = self.buffer.line_to_byte(line) + self.buffer.line_text(line).len()
            - self.buffer.line_text(line).trim_start().len();
        let mut node = tree.root_node().descendant_for_byte_range(byte, byte)?;
        loop {
            if RUST_SCOPES.contains(&node.kind()) {
                break;
            }
            node = node.parent()?;
        }
        let mut start = node.start_position().row;
        let end = node.end_position().row + 1;
        // Bring the doc comments and attributes just above along.
        while start > 0 {
            let above = self.buffer.line_text(start - 1);
            let above = above.trim_start();
            if above.starts_with("///") || above.starts_with("#[") || above.starts_with("//!") {
                start -= 1;
            } else {
                break;
            }
        }
        Some(start..end.min(self.buffer.len_lines()))
    }

    fn line_range_chars(&self, lines: &Range<usize>) -> Range<usize> {
        let start = self.buffer.line_to_char(lines.start);
        let end = if lines.end >= self.buffer.len_lines() {
            self.buffer.len_chars()
        } else {
            self.buffer.line_to_char(lines.end)
        };
        start..end
    }

    /// ⌘I. With a change showing, it goes back to the field to adjust it.
    pub(crate) fn open_inline_assist(&mut self, ask_only: bool, window: &mut Window, cx: &mut Context<Self>) {
        // AI switched off: no AI anywhere, not even a field saying so.
        if !cx.global::<Settings>().ai.enabled {
            return;
        }
        if let Some(prompt) = &self.prompt {
            return window.focus(&prompt.input.focus_handle(cx));
        }
        let mut previous = String::new();
        let mut lines = None;
        if let Some(change) = self.ai_change.take() {
            // Adjust: put the code back as it was and ask again, starting from the last request.
            self.step_history(true, cx);
            previous = change.instruction;
            lines = Some(change.lines);
        }
        self.close_completion(cx);
        self.close_hover(cx);
        let lines = lines.unwrap_or_else(|| {
            if ask_only {
                self.selected_line_span().unwrap_or_else(|| {
                    let (line, _) = self.caret_point();
                    line..line + 1
                })
            } else {
                self.assist_lines()
            }
        });
        let span = self.line_range_chars(&lines);
        let error = self
            .problems(cx)
            .iter()
            .filter(|p| p.severity == DiagnosticSeverity::ERROR)
            .find(|p| p.range.start < span.end && p.range.end >= span.start)
            .map(|p| p.message.clone());
        let placeholder = match (&error, ask_only) {
            (_, true) => "Ask about this code",
            (Some(_), false) => "Fix the error, or describe a change",
            (None, false) => "Describe a change, or ask a question",
        };
        let input = cx.new(|cx| TextInput::new(placeholder, cx));
        if !previous.is_empty() {
            input.update(cx, |input, cx| input.set_text(&previous, cx));
        }
        let subscription = cx.subscribe(&input, |this, _, TextInputEvent::Changed, cx| {
            if let Some(prompt) = &mut this.prompt {
                prompt.failed = None;
            }
            cx.notify();
        });
        window.focus(&input.focus_handle(cx));
        self.prompt = Some(Prompting {
            input,
            original: self.buffer.slice(span),
            lines,
            error: if ask_only { None } else { error },
            ask_only,
            writing: false,
            preview: String::new(),
            failed: None,
            task: None,
            _subscription: subscription,
        });
        self.rebuild_blocks();
        self.autoscroll = true;
        cx.notify();
    }

    /// ⌘.'s Fix with AI: ⌘I's field on the error's code, sent at once with "Fix this error".
    pub(super) fn fix_with_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_inline_assist(false, window, cx);
        if self.prompt.as_ref().is_some_and(|p| p.error.is_some()) {
            self.submit_prompt(&SubmitPrompt, window, cx);
        }
    }

    fn submit_prompt(&mut self, _: &SubmitPrompt, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = &self.prompt else { return };
        if prompt.writing {
            return;
        }
        let typed = prompt.input.read(cx).text().trim().to_string();
        let instruction = match (&prompt.error, typed.is_empty()) {
            (_, false) => typed.clone(),
            (Some(error), true) => format!("Fix this error: {error}"),
            (None, true) => return,
        };
        if prompt.ask_only || is_question(&typed) {
            return self.ask_question(instruction, window, cx);
        }
        // The field goes while the change is written: the keys come here (Esc stops it, ⇥
        // and Esc keep or undo what's written).
        window.focus(&self.focus_handle);
        self.write_change(instruction, cx);
    }

    pub(super) fn cancel_prompt(&mut self, _: &CancelPrompt, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt = None;
        self.rebuild_blocks();
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// Text hidden in the file (see `invisible::without_hidden`) isn't sent: said.
    fn say_hidden_left_out(&mut self, cx: &mut Context<Self>) {
        let hidden = super::invisible::without_hidden(&self.buffer.to_string()).1;
        if hidden > 0 {
            let message = format!("Hidden text in this file ({hidden} characters) is left out of what the AI reads.");
            self.show_notice(self.selection.head, message, cx);
        }
    }

    fn file_context(&self, lines: &Range<usize>) -> String {
        let path = self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "untitled".into());
        format!(
            "File: {path} ({})\n\n<file>\n{}\n</file>",
            self.language_name(),
            trim_context(&self.buffer.to_string(), lines)
        )
    }

    fn write_change(&mut self, instruction: String, cx: &mut Context<Self>) {
        let Some(prompt) = &self.prompt else { return };
        let lines = prompt.lines.clone();
        let user = format!(
            "{}\n\n<selection lines=\"{}-{}\">\n{}</selection>\n\nInstruction: {instruction}",
            self.file_context(&lines),
            lines.start + 1,
            lines.end,
            prompt.original
        );
        let request = Prompt {
            system: "You are a careful senior engineer making a small change to part of a file in a code editor. \
                     You get the whole file for context and the part to change between <selection> tags. Apply the \
                     instruction to that part only. Reply with the complete new text for that part and nothing \
                     else: no explanation, no markdown fences, nothing before or after the code. Keep the file's \
                     indentation and style, and change no more than needed."
                .into(),
            user,
            ..Default::default()
        };
        self.say_hidden_left_out(cx);
        let mut events = ai::stream(cx.global::<Settings>().ai.clone(), request);
        let task = cx.spawn(async move |this, cx| {
            let mut text = String::new();
            while let Some(event) = events.next().await {
                match event {
                    AiEvent::Text(chunk) => {
                        text.push_str(&chunk);
                        let so_far = text.clone();
                        this.update(cx, |this, cx| {
                            if let Some(prompt) = &mut this.prompt {
                                prompt.preview = so_far;
                            }
                            this.rebuild_blocks();
                            cx.notify();
                        })
                        .ok();
                    }
                    AiEvent::Done => {
                        let text = std::mem::take(&mut text);
                        let instruction = instruction.clone();
                        this.update(cx, |this, cx| this.apply_change(text, instruction, cx)).ok();
                        break;
                    }
                    AiEvent::Failed(message) => {
                        this.update(cx, |this, cx| {
                            if let Some(prompt) = &mut this.prompt {
                                prompt.writing = false;
                                prompt.failed = Some(message);
                                this.focus_prompt = true;
                            }
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                }
            }
        });
        if let Some(prompt) = &mut self.prompt {
            prompt.writing = true;
            prompt.task = Some(task);
        }
        cx.notify();
    }

    /// Writes the change into the file, as one undo step, and marks what changed.
    fn apply_change(&mut self, answer: String, instruction: String, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt.take() else { return };
        let mut text = ai::strip_code_fence(&answer);
        if prompt.original.ends_with('\n') && !text.ends_with('\n') {
            text.push('\n');
        }
        if text.trim().is_empty() || text == prompt.original {
            let failed = if text.trim().is_empty() {
                "The answer was empty. Try rephrasing."
            } else {
                "Nothing to change there."
            };
            self.prompt = Some(Prompting { writing: false, failed: Some(failed.into()), task: None, ..prompt });
            self.focus_prompt = true;
            return cx.notify();
        }
        let lines = prompt.lines.clone();
        let range = self.line_range_chars(&lines);
        // The lines asked about changed meanwhile (typed into, moved by an edit above, read
        // again from disk): the answer isn't written over what's there now.
        if self.buffer.slice(range.clone()) != prompt.original {
            let failed = "The code changed while the answer came. Ask again.";
            self.prompt = Some(Prompting { writing: false, failed: Some(failed.into()), task: None, ..prompt });
            self.focus_prompt = true;
            return cx.notify();
        }
        self.record_undo(EditKind::Other);
        self.buffer.replace(range.clone(), &text);

        let old: Vec<&str> = prompt.original.lines().collect();
        let diff = similar::TextDiff::from_lines(prompt.original.as_str(), text.as_str());
        let mut removed = Vec::new();
        let mut added = Vec::new();
        for op in diff.ops() {
            match *op {
                similar::DiffOp::Equal { .. } => {}
                similar::DiffOp::Delete { old_index, old_len, new_index } => {
                    removed.push((
                        lines.start + new_index,
                        old[old_index..old_index + old_len].iter().map(|s| s.to_string()).collect(),
                    ));
                }
                similar::DiffOp::Insert { new_index, new_len, .. } => {
                    added.push(lines.start + new_index..lines.start + new_index + new_len);
                }
                similar::DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                    removed.push((
                        lines.start + new_index,
                        old[old_index..old_index + old_len].iter().map(|s| s.to_string()).collect(),
                    ));
                    added.push(lines.start + new_index..lines.start + new_index + new_len);
                }
            }
        }
        let new_lines = text.lines().count();
        let first_change = removed.first().map(|(l, _)| *l).into_iter().chain(added.first().map(|r| r.start)).min();
        self.single_cursor();
        self.selection = Selection::caret(self.buffer.line_to_char(first_change.unwrap_or(lines.start)));
        self.goal_column = None;
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        self.ai_change = Some(Change {
            version: self.buffer.version(),
            added,
            removed,
            hint_line: lines.start + new_lines,
            lines: lines.start..lines.start + new_lines,
            instruction,
        });
        self.rebuild_blocks();
        self.touch(cx);
        cx.notify();
    }

    fn keep_change(&mut self, _: &KeepChange, _: &mut Window, cx: &mut Context<Self>) {
        self.ai_change = None;
        self.rebuild_blocks();
        cx.notify();
    }

    fn undo_change(&mut self, _: &UndoChange, _: &mut Window, cx: &mut Context<Self>) {
        if self.ai_change.take().is_some() {
            self.step_history(true, cx);
        }
        self.rebuild_blocks();
        cx.notify();
    }

    fn ask_question(&mut self, question: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt.take() else { return };
        let lines = prompt.lines.clone();
        let user = format!(
            "{}\n\nThe code in question (lines {}-{}):\n<code>\n{}</code>\n\nQuestion: {question}",
            self.file_context(&lines),
            lines.start + 1,
            lines.end,
            prompt.original
        );
        let request = Prompt {
            system: "You are a concise, knowledgeable programming assistant inside a code editor. Your answer is \
                     shown as a small note right under the code it's about, so keep it short: a few sentences, \
                     specific to this code. Use a markdown code fence only when code really helps."
                .into(),
            user,
            ..Default::default()
        };
        self.say_hidden_left_out(cx);
        let mut events = ai::stream(cx.global::<Settings>().ai.clone(), request);
        let task = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let done = this
                    .update(cx, |this, cx| {
                        let Some(note) = &mut this.note else { return true };
                        let done = match event {
                            AiEvent::Text(text) => {
                                note.answer.push_str(&text);
                                false
                            }
                            AiEvent::Done => true,
                            AiEvent::Failed(message) => {
                                note.failed = Some(message);
                                true
                            }
                        };
                        note.done = done;
                        this.rebuild_blocks();
                        cx.notify();
                        done
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        });
        self.note = Some(Note {
            line: lines.end,
            question,
            answer: String::new(),
            done: false,
            failed: None,
            scroll: ScrollHandle::new(),
            lines_total: self.buffer.len_lines(),
            _task: Some(task),
        });
        self.rebuild_blocks();
        window.focus(&self.focus_handle);
        self.autoscroll = true;
        cx.notify();
    }

    fn close_note(&mut self, _: &CloseNote, _: &mut Window, cx: &mut Context<Self>) {
        self.note = None;
        self.rebuild_blocks();
        cx.notify();
    }

    /// After the text changes: a change being shown counts as kept once edited by hand,
    /// and a note follows lines added or removed above it.
    pub(super) fn ai_text_changed(&mut self) {
        if self.ai_change.as_ref().is_some_and(|c| c.version != self.buffer.version()) {
            self.ai_change = None;
        }
        let total = self.buffer.len_lines();
        let caret_line = self.buffer.point(self.selection.head).0;
        if let Some(note) = &mut self.note {
            let delta = total as isize - note.lines_total as isize;
            if delta != 0 && caret_line < note.line {
                note.line = (note.line as isize + delta).clamp(0, total as isize) as usize;
            }
            note.lines_total = total;
        }
        self.rebuild_blocks();
    }

    /// The rows the note needs: roughly, its text wrapped to the note's width.
    fn note_rows(&self, note: &Note) -> usize {
        let per_row = self
            .layout
            .as_ref()
            .map(|l| ((l.text_bounds.size.width - px(60.)) / (l.char_width * 0.95)).floor().max(20.) as usize)
            .unwrap_or(80);
        let text = if let Some(failed) = &note.failed { failed.as_str() } else { note.answer.as_str() };
        // Text lines wrapped to the width; blank lines between paragraphs are only small gaps.
        let body: usize =
            text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.chars().count().div_ceil(per_row)).sum();
        let gaps = text.lines().filter(|l| l.trim().is_empty()).count().div_ceil(2);
        (body + gaps + 1).clamp(2, NOTE_MAX_ROWS)
    }

    pub(super) fn rebuild_blocks(&mut self) {
        self.refresh_review();
        let mut blocks = Vec::new();
        if let Some(prompt) = &self.prompt {
            blocks.push(Block { before_line: prompt.lines.start, rows: 1, kind: BlockKind::Prompt });
            // The new code appears line by line as it's written, above the code it replaces.
            let written: Vec<String> =
                prompt.preview.lines().filter(|l| !l.trim_start().starts_with("```")).map(str::to_string).collect();
            if prompt.writing && !written.is_empty() {
                blocks.push(Block {
                    before_line: prompt.lines.start,
                    rows: written.len(),
                    kind: BlockKind::Writing(written),
                });
            }
        }
        if let Some(change) = &self.ai_change {
            for (line, old) in &change.removed {
                blocks.push(Block { before_line: *line, rows: old.len(), kind: BlockKind::Removed(old.clone()) });
            }
            blocks.push(Block { before_line: change.hint_line, rows: 1, kind: BlockKind::Hint });
        }
        if let Some(note) = &self.note {
            let rows = self.note_rows(note);
            blocks.push(Block { before_line: note.line, rows, kind: BlockKind::Note });
        }
        // An AI task's changes being reviewed: old lines struck through above the new ones,
        // and the keys under the change at the caret.
        if let Some(review) = &self.review {
            for hunk in &review.hunks {
                if !hunk.old_lines.is_empty() {
                    blocks.push(Block {
                        before_line: hunk.new.start,
                        rows: hunk.old_lines.len(),
                        kind: BlockKind::Removed(hunk.old_lines.clone()),
                    });
                }
            }
            if let Some(index) = self.current_hunk() {
                let hunk = &review.hunks[index];
                blocks.push(Block {
                    before_line: hunk.new.end.max(hunk.new.start),
                    rows: 1,
                    kind: BlockKind::ReviewHint(review.hunks.len() - 1),
                });
            }
        }
        if let (Some((_, rest)), Some(line)) = (self.ghost_text(), self.ghost_line())
            && !rest.is_empty()
        {
            blocks.push(Block { before_line: line + 1, rows: rest.len(), kind: BlockKind::Ghost(rest.to_vec()) });
        }
        self.blocks = blocks;
    }

    pub fn block_specs(&self) -> Vec<BlockSpec> {
        self.blocks.iter().map(|b| BlockSpec { before_line: b.before_line, rows: b.rows }).collect()
    }

    /// Lines tinted while ⌘I is open: the code it will change.
    pub fn assist_target(&self) -> Option<Range<usize>> {
        self.prompt.as_ref().map(|p| p.lines.clone())
    }

    /// The code being rewritten, dimmed while the AI writes its replacement.
    pub fn ai_writing_lines(&self) -> Option<Range<usize>> {
        self.prompt.as_ref().filter(|p| p.writing).map(|p| p.lines.clone())
    }

    pub fn ai_added_lines(&self) -> Vec<Range<usize>> {
        let mut lines = self.ai_change.as_ref().map_or(Vec::new(), |c| c.added.clone());
        lines.extend(self.review_added_lines());
        lines
    }

    /// See `focus_prompt`.
    pub(super) fn focus_prompt_if_asked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.focus_prompt)
            && let Some(prompt) = self.prompt.as_ref().filter(|p| !p.writing)
        {
            window.focus(&prompt.input.focus_handle(cx));
        }
    }

    pub(super) fn ai_key_context(&self, context: &mut gpui::KeyContext) {
        if self.ai_change.is_some() {
            context.add("ai_change");
        }
        if self.prompt.as_ref().is_some_and(|p| p.writing) {
            context.add("ai_writing");
        }
        if self.review.is_some() {
            context.add("review");
        }
        if self.note.is_some() {
            context.add("ai_note");
        }
        // The language server's list, when open, keeps ⇥ for itself.
        if self.ghost.is_some() && self.completion.is_none() {
            context.add("ai_ghost");
        }
    }

    /// The field, the keys under a change, and the note, each on its rows.
    pub(super) fn render_ai_blocks(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(layout) = &self.layout else { return Vec::new() };
        let theme = cx.global::<Theme>().clone();
        let lh = layout.line_height;
        let left = layout.text_bounds.left() - layout.bounds.left() + px(4.);
        let width = layout.text_bounds.size.width - px(28.);
        let mut out = Vec::new();
        for (i, block) in self.blocks.iter().enumerate() {
            let Some(row) = self.wrap.block_row(i) else { continue };
            let top = layout.text_origin.y + lh * row as f32 - layout.bounds.top();
            let place = |d: gpui::Div| d.absolute().top(top).left(left).w(width).h(lh * block.rows as f32);
            match &block.kind {
                BlockKind::Prompt => {
                    let Some(prompt) = &self.prompt else { continue };
                    let (status, hint): (Option<String>, &str) = if prompt.writing {
                        (Some("Writing…".into()), "esc cancel")
                    } else if let Some(failed) = &prompt.failed {
                        (None, if failed.len() > 60 { "" } else { failed.as_str() })
                    } else {
                        let typed = prompt.input.read(cx).text().trim().to_string();
                        let hint = if typed.is_empty() {
                            if prompt.error.is_some() { "↵ fix error" } else { "" }
                        } else if prompt.ask_only || is_question(&typed) {
                            "↵ ask"
                        } else {
                            "↵ change"
                        };
                        (None, hint)
                    };
                    let failed_long = prompt.failed.as_ref().filter(|f| f.len() > 60).cloned();
                    out.push(
                        place(div())
                            .key_context("AiPrompt")
                            .on_action(cx.listener(Self::submit_prompt))
                            .on_action(cx.listener(Self::cancel_prompt))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(10.))
                            .rounded(px(ui::R_CONTROL))
                            .border_1()
                            .border_color(if prompt.writing { theme.caret.opacity(0.5) } else { theme.hairline })
                            .bg(theme.raised)
                            .text_size(px(13.))
                            .child(div().text_color(theme.caret).child("✦"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_color(theme.foreground)
                                    .children(status.map(|s| div().text_color(theme.muted).child(s)))
                                    .when(!prompt.writing, |d| d.child(prompt.input.clone())),
                            )
                            .children(failed_long.map(|f| {
                                div()
                                    .max_w(px(360.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_color(theme.error)
                                    .child(f)
                            }))
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(ui::T_XS))
                                    .text_color(if prompt.failed.is_some() { theme.error } else { theme.muted })
                                    .child(hint.to_string()),
                            )
                            .into_any_element(),
                    );
                }
                BlockKind::Hint => {
                    // The keys as they're bound now, so a changed shortcut shows here too.
                    let key = |action: &dyn gpui::Action, fallback: &str, label: &str| {
                        let keys = crate::palette::shortcut(action, cx).unwrap_or_else(|| fallback.to_string());
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(ui::key_cap(keys, &theme))
                            .child(label.to_string())
                    };
                    out.push(
                        place(div())
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .text_size(px(ui::T_XS))
                            .text_color(theme.muted)
                            .child(key(&KeepChange, "⇥", "keep"))
                            .child(key(&UndoChange, "Esc", "undo"))
                            .child(key(&super::InlineAssist, "⌘I", "adjust"))
                            .into_any_element(),
                    );
                }
                BlockKind::ReviewHint(more) => {
                    // The keys as they're bound now, so a changed shortcut shows here too.
                    let key = |action: &dyn gpui::Action, fallback: &str, label: &str| {
                        let keys = crate::palette::shortcut(action, cx).unwrap_or_else(|| fallback.to_string());
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(ui::key_cap(keys, &theme))
                            .child(label.to_string())
                    };
                    out.push(
                        place(div())
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .text_size(px(ui::T_XS))
                            .text_color(theme.muted)
                            .child(key(&super::KeepHunk, "⇥", "keep"))
                            .child(key(&super::UndoHunk, "Esc", "undo"))
                            .when(*more > 0, |d| {
                                d.child(if *more == 1 {
                                    "1 more change".to_string()
                                } else {
                                    format!("{more} more changes")
                                })
                            })
                            .into_any_element(),
                    );
                }
                BlockKind::Note => {
                    let Some(note) = &self.note else { continue };
                    let body: Vec<AnyElement> = if let Some(failed) = &note.failed {
                        vec![div().text_color(theme.error).child(failed.clone()).into_any_element()]
                    } else if note.answer.trim().is_empty() {
                        vec![div().text_color(theme.muted).child("Thinking…").into_any_element()]
                    } else {
                        crate::markdown::blocks(&note.answer, None)
                            .into_iter()
                            .map(|b| {
                                if b.code {
                                    div()
                                        .my(px(4.))
                                        .px(px(10.))
                                        .py(px(6.))
                                        .rounded(px(ui::R_ROW))
                                        .bg(theme.sunken)
                                        .code_font(cx)
                                        .text_size(px(12.5))
                                        .children(
                                            b.text
                                                .lines()
                                                .map(|l| div().whitespace_nowrap().child(l.to_string()))
                                                .collect::<Vec<_>>(),
                                        )
                                        .into_any_element()
                                } else {
                                    div().py(px(2.)).child(b.text).into_any_element()
                                }
                            })
                            .collect()
                    };
                    out.push(
                        place(div())
                            .py(px(3.))
                            .child(
                                div()
                                    .id("ai-note")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&note.scroll)
                                    .border_l_2()
                                    .border_color(theme.caret)
                                    .bg(theme.surface)
                                    .px(px(14.))
                                    .py(px(6.))
                                    .text_size(px(13.))
                                    .line_height(px(20.))
                                    .text_color(theme.foreground)
                                    .child(
                                        div()
                                            .flex()
                                            .justify_between()
                                            .text_size(px(ui::T_XS))
                                            .text_color(theme.muted)
                                            .child(
                                                div()
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .child(note.question.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .pl(px(12.))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(if note.done { "esc close" } else { "" }),
                                            ),
                                    )
                                    .children(body),
                            )
                            .into_any_element(),
                    );
                }
                BlockKind::Removed(_) | BlockKind::Ghost(_) | BlockKind::Writing(_) => {}
            }
        }
        out
    }

    pub(super) fn keep_change_action(&mut self, a: &KeepChange, w: &mut Window, cx: &mut Context<Self>) {
        self.keep_change(a, w, cx)
    }

    pub(super) fn undo_change_action(&mut self, a: &UndoChange, w: &mut Window, cx: &mut Context<Self>) {
        self.undo_change(a, w, cx)
    }

    pub(super) fn close_note_action(&mut self, a: &CloseNote, w: &mut Window, cx: &mut Context<Self>) {
        self.close_note(a, w, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use gpui::{EntityInputHandler as _, TestAppContext, VisualTestContext};
    use std::path::PathBuf;

    fn editor<'a>(cx: &'a mut TestAppContext, text: &str) -> (Entity<Editor>, &'a mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = text.to_string();
        cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(PathBuf::from("x.py")), cx))
    }

    #[gpui::test]
    fn a_change_is_written_in_shown_as_a_diff_and_undone(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "def f(x):\n    return x / 0\n\nprint(f(1))\n");
        e.update_in(cx, |e, window, cx| {
            e.selection = Selection::caret(2);
            e.open_inline_assist(false, window, cx);
            // The field sits above the paragraph it changes.
            assert!(matches!(e.blocks.first(), Some(Block { before_line: 0, kind: BlockKind::Prompt, .. })));
            e.apply_change("def f(x):\n    return x / 2\n".into(), "halve".into(), cx);
            assert_eq!(e.buffer.to_string(), "def f(x):\n    return x / 2\n\nprint(f(1))\n");
            // One line removed (shown struck through above its replacement), one added.
            assert_eq!(e.ai_added_lines(), vec![1..2]);
            assert!(e.blocks.iter().any(|b| matches!(&b.kind, BlockKind::Removed(l) if l == &["    return x / 0"])));
            e.undo_change(&UndoChange, window, cx);
            assert_eq!(e.buffer.to_string(), "def f(x):\n    return x / 0\n\nprint(f(1))\n");
            assert!(e.blocks.is_empty());
            // A line added above while the answer came: it isn't written over the wrong lines.
            e.selection = Selection::caret(2);
            e.open_inline_assist(false, window, cx);
            e.buffer.replace(0..0, "import os\n");
            e.apply_change("def f(x):\n    return x / 2\n".into(), "halve".into(), cx);
            assert_eq!(e.buffer.to_string(), "import os\ndef f(x):\n    return x / 0\n\nprint(f(1))\n");
            assert!(e.prompt.as_ref().is_some_and(|p| p.failed.is_some()), "said, not applied");
        });
    }

    #[gpui::test]
    fn a_ghost_completion_is_taken_with_tab_or_dropped_by_moving(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "x = 1\ny = \n");
        e.update_in(cx, |e, window, cx| {
            e.selection = Selection::caret(10);
            let ghost = |e: &Editor| {
                super::super::ghost::Ghost::new(10, e.buffer.version(), vec!["x + 1".into(), "z = 3".into()])
            };
            e.ghost = Some(ghost(e));
            e.rebuild_blocks();
            // Its second line sits below the caret's line.
            assert!(matches!(&e.blocks[..], [Block { before_line: 2, kind: BlockKind::Ghost(_), .. }]));
            e.accept_ghost_action(&super::super::ghost::AcceptGhost, window, cx);
            assert_eq!(e.buffer.to_string(), "x = 1\ny = x + 1\nz = 3\n");
            assert!(e.blocks.is_empty());
            // Moving the caret away drops a ghost.
            e.ghost = Some(super::super::ghost::Ghost::new(e.selection.head, e.buffer.version(), vec!["!".into()]));
            e.move_head(0, false, cx);
            assert!(e.ghost.is_none());
        });
    }

    /// Typing part of a name defined above suggests the rest right away, and typing
    /// along keeps the suggestion (no AI involved: the provider here is off).
    #[gpui::test]
    fn names_from_the_file_are_suggested_while_typing(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "def compute_moving_average(xs):\n    return 0\n\n");
        cx.update(|_, cx| {
            let mut settings = Settings::default();
            settings.ai.completions = true;
            cx.set_global(settings);
        });
        e.update_in(cx, |e, window, cx| {
            e.selection = Selection::caret(e.buffer.len_chars());
            for c in "res = comp".chars() {
                e.replace_text_in_range(None, &c.to_string(), window, cx);
            }
            assert_eq!(e.ghost_text().map(|(first, _)| first), Some("ute_moving_average"));
            e.replace_text_in_range(None, "u", window, cx);
            assert_eq!(e.ghost_text().map(|(first, _)| first), Some("te_moving_average"));
            // ⌥→ takes one word at a time; the rest stays suggested.
            e.selection = Selection::caret(e.buffer.len_chars());
            e.ghost = Some(super::super::ghost::Ghost::new(
                e.selection.head,
                e.buffer.version(),
                vec!["te_moving_average(xs)".into()],
            ));
            e.accept_ghost_word(&super::super::ghost::AcceptGhostWord, window, cx);
            assert!(e.buffer.to_string().ends_with("res = compute_moving_average"));
            assert_eq!(e.ghost_text().map(|(first, _)| first), Some("(xs)"));
        });
    }

    #[gpui::test]
    fn editing_by_hand_keeps_the_change(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "a = 1\n");
        e.update_in(cx, |e, window, cx| {
            e.open_inline_assist(false, window, cx);
            e.apply_change("a = 2\n".into(), "two".into(), cx);
            assert!(!e.blocks.is_empty());
            e.edit(0..0, "# ", EditKind::Typing, cx);
            assert!(e.blocks.is_empty());
            assert_eq!(e.buffer.to_string(), "# a = 2\n");
        });
    }

    #[test]
    fn questions_are_told_apart_from_changes() {
        assert!(is_question("why is this slow?"));
        assert!(is_question("Explain the borrow here"));
        assert!(!is_question("make it return a Result"));
        assert!(!is_question("add a test"));
    }
}
