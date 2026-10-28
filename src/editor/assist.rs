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

actions!(inline_ai, [SubmitPrompt, CancelPrompt, KeepChange, UndoChange, CloseNote, AskFollowUp, CloseFollowUp]);

/// Esc while the AI writes, with the keys in the code (a click there): it stops it. After
/// the review's and the suggestions' keys, so it's this Esc that counts then.
pub fn bind_writing_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CancelPrompt, Some("Editor && ai_writing"))]);
}

/// Registered after the editor's own keys, so ⇥ and Esc mean keep and undo while a change is shown.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", SubmitPrompt, Some("AiPrompt")),
        KeyBinding::new("escape", CancelPrompt, Some("AiPrompt")),
        KeyBinding::new("tab", KeepChange, Some("Editor && ai_change")),
        KeyBinding::new("escape", UndoChange, Some("Editor && ai_change")),
        KeyBinding::new("escape", CloseNote, Some("Editor && ai_note")),
        KeyBinding::new("enter", AskFollowUp, Some("NoteFollowUp")),
        KeyBinding::new("escape", CloseFollowUp, Some("NoteFollowUp")),
    ]);
}

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
    /// The keys for a reviewed change, with how many more there are, and whether an AI
    /// task made it (undone) or not (discarded).
    ReviewHint(usize, bool),
    /// A conversation on the line above (`threads`' index).
    Thread(usize),
    /// A note on the line above (`line_notes`' index).
    LineNote(usize),
    /// A note being written on the line above.
    NoteDraft,
}

/// A conversation between the lines: a pull request's review comments on a line.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub id: u64,
    /// The line it's on (0-based): it's shown under it.
    pub line: usize,
    /// Each comment: who and when, then what it says.
    pub comments: Vec<(String, String)>,
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
    /// What's sent with the request besides the code itself (see `ai_context`), in a few
    /// words, with its size: shown in the field.
    sends: String,
    /// Opened to ask about the code, not to change it.
    ask_only: bool,
    writing: bool,
    /// What the AI has written so far, shown as it arrives.
    preview: String,
    failed: Option<String>,
    task: Option<Task<()>>,
    /// Rows the field takes beyond its first, with several lines typed.
    added_rows: usize,
    _subscriptions: [Subscription; 2],
}

/// A change written into the file, until it's kept or undone.
pub(super) struct Change {
    version: u64,
    pub(super) added: Vec<Range<usize>>,
    removed: Vec<(usize, Vec<String>)>,
    hint_line: usize,
    /// The lines asked about, before the change.
    lines: Range<usize>,
    instruction: String,
}

pub(super) struct Note {
    /// The note sits just above this line, under the code it's about.
    line: usize,
    /// The code it's about, as asked.
    code_lines: Range<usize>,
    code: String,
    /// The questions asked before this one, with their answers: a follow-up goes with them.
    earlier: Vec<(String, String)>,
    /// What else the question goes with (a failed test run's output).
    extra: String,
    question: String,
    answer: String,
    /// A follow-up question, once the answer's there.
    follow_up: Entity<TextInput>,
    /// Rows it takes beyond its first, with several lines typed.
    follow_up_rows: usize,
    _follow_up_resized: Subscription,
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

impl Editor {
    /// The lines ⌘I changes: the selected lines, else the function (class, CSS rule,
    /// HTML element…) around the caret (see `ai_scope`), else the paragraph around it; no
    /// more than `ai_scope::MAX_LINES` of either (else the caret's line).
    fn assist_lines(&mut self) -> Range<usize> {
        if let Some(lines) = self.selected_line_span() {
            return lines;
        }
        let (caret_line, _) = self.caret_point();
        let language = self.language_name();
        let tree = match &mut self.highlighter {
            Some(h) => h.ready(&self.buffer).then(|| h.tree().cloned()).flatten(),
            None => None,
        };
        let line = |l: usize| self.buffer.line_text(l).to_string();
        if let Some(lines) = tree.and_then(|tree| super::ai_scope::unit_lines(&tree, language, caret_line, &line)) {
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
        if end - start > super::ai_scope::MAX_LINES { caret_line..caret_line + 1 } else { start..end }
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
        // AI switched off (or no provider to ask): no AI anywhere, not even a field saying so.
        if !cx.global::<Settings>().ai.ready() {
            return;
        }
        if let Some(prompt) = &self.prompt {
            let keys = if prompt.writing { self.writing_focus.clone() } else { prompt.input.focus_handle(cx) };
            return window.focus(&keys);
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
        let sends = {
            let context = self.request_context(&lines, cx);
            let code = self.buffer.slice(self.line_range_chars(&lines));
            let tokens = crate::ai_usage::estimate(&context.text) + crate::ai_usage::estimate(&code) + 150;
            format!("with {} · about {} tokens", context.summary, crate::ai_usage::short(tokens))
        };
        let input = cx.new(|cx| TextInput::new(placeholder, cx).multiline());
        if !previous.is_empty() {
            input.update(cx, |input, cx| input.set_text(&previous, cx));
        }
        let subscription = cx.subscribe(&input, |this, _, TextInputEvent::Changed, cx| {
            if let Some(prompt) = &mut this.prompt {
                prompt.failed = None;
            }
            cx.notify();
        });
        // More lines typed: room made for them.
        let resized = cx.observe(&input, |this, input, cx| {
            let rows = input.read(cx).rows_added(this.line_height());
            if let Some(prompt) = this.prompt.as_mut().filter(|p| p.added_rows != rows) {
                prompt.added_rows = rows;
                this.rebuild_blocks();
                cx.notify();
            }
        });
        window.focus(&input.focus_handle(cx));
        self.prompt = Some(Prompting {
            input,
            original: self.buffer.slice(span),
            lines,
            error: if ask_only { None } else { error },
            sends,
            ask_only,
            writing: false,
            preview: String::new(),
            failed: None,
            task: None,
            added_rows: 0,
            _subscriptions: [subscription, resized],
        });
        self.rebuild_blocks();
        self.autoscroll = true;
        cx.notify();
    }

    /// ⌘I on what's selected, sent at once with `instruction` (shown in the field).
    pub(super) fn change_with_ai(&mut self, instruction: String, window: &mut Window, cx: &mut Context<Self>) {
        self.open_inline_assist(false, window, cx);
        let Some(prompt) = &self.prompt else { return };
        prompt.input.update(cx, |input, cx| input.set_text(&instruction, cx));
        self.submit_prompt(&SubmitPrompt, window, cx);
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
        // The field goes while the change is written: the keys go to its "Writing…" bar,
        // where Esc stops it and nothing typed reaches the code being rewritten.
        window.focus(&self.writing_focus);
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

    /// What the AI reads with a request about `lines`, besides them: the project's
    /// conventions, the file (or the part of it around them), what they use from elsewhere.
    fn request_context(&self, lines: &Range<usize>, cx: &App) -> crate::ai_context::Context {
        let project = cx.try_global::<crate::project_index::ProjectContext>();
        let root = project.map(|p| p.root.as_path()).filter(|r| !r.as_os_str().is_empty());
        crate::ai_context::for_request(
            self.path.as_deref(),
            self.language_name(),
            &self.buffer.to_string(),
            lines,
            root,
            project.map_or(&[][..], |p| p.definitions.as_slice()),
            cx.global::<Settings>().ai.active(),
        )
    }

    fn write_change(&mut self, instruction: String, cx: &mut Context<Self>) {
        let Some(prompt) = &self.prompt else { return };
        let lines = prompt.lines.clone();
        let user = format!(
            "<selection lines=\"{}-{}\">\n{}</selection>\n\nInstruction: {instruction}",
            lines.start + 1,
            lines.end,
            prompt.original
        );
        let request = Prompt {
            system: "You are a careful senior engineer making a small change to part of a file in a code editor. \
                     You get the file for context (or, in a long one, the lines around the change and an outline of \
                     the rest), what the code uses from elsewhere in the project, and the part to change between \
                     <selection> tags. Apply the instruction to that part only. Reply with the complete new text \
                     for that part and nothing else: no explanation, no markdown fences, nothing before or after the \
                     code. Keep the file's indentation and style, and change no more than needed."
                .into(),
            context: self.request_context(&lines, cx).text,
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
        // A small model sends back code around the part asked about too (the function, or
        // the whole file): what repeats the lines above it and below it is left out.
        let asked = &prompt.lines;
        let before: Vec<String> = (0..asked.start).map(|l| self.buffer.line_text(l)).collect();
        let after: Vec<String> = (asked.end..self.buffer.len_lines()).map(|l| self.buffer.line_text(l)).collect();
        text = without_context(&text, &before, &after, asked.len());
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
            return cx.notify();
        }
        let lines = prompt.lines.clone();
        let range = self.line_range_chars(&lines);
        // The lines asked about changed meanwhile (typed into, moved by an edit above, read
        // again from disk): the answer isn't written over what's there now.
        if self.buffer.slice(range.clone()) != prompt.original {
            let failed = "The code changed before the answer arrived. Ask again.";
            self.prompt = Some(Prompting { writing: false, failed: Some(failed.into()), task: None, ..prompt });
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
            // (As they were: adjusting puts them back first.)
            lines,
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
        let (follow_up, resized) = Self::follow_up_field(cx);
        self.note = Some(Note {
            line: prompt.lines.end,
            code_lines: prompt.lines.clone(),
            code: prompt.original.clone(),
            earlier: Vec::new(),
            extra: String::new(),
            question: question.clone(),
            answer: String::new(),
            follow_up,
            follow_up_rows: 0,
            _follow_up_resized: resized,
            done: false,
            failed: None,
            scroll: ScrollHandle::new(),
            lines_total: self.buffer.len_lines(),
            _task: None,
        });
        self.send_note_question(cx);
        window.focus(&self.focus_handle);
        self.autoscroll = true;
    }

    /// A note's field for a follow-up question, making room for the lines typed in it.
    fn follow_up_field(cx: &mut Context<Self>) -> (Entity<TextInput>, Subscription) {
        let input = cx.new(|cx| TextInput::new("Ask a follow-up", cx).multiline());
        let resized = cx.observe(&input, |this, input, cx| {
            let rows = input.read(cx).rows_added(this.line_height());
            if let Some(note) = this.note.as_mut().filter(|n| n.follow_up_rows != rows) {
                note.follow_up_rows = rows;
                this.rebuild_blocks();
                cx.notify();
            }
        });
        (input, resized)
    }

    /// Asks the note's question, with what was asked before it, and writes the answer in.
    fn send_note_question(&mut self, cx: &mut Context<Self>) {
        let Some(note) = &self.note else { return };
        let context = self.request_context(&note.code_lines, cx).text;
        let earlier: String =
            note.earlier.iter().map(|(q, a)| format!("Question: {q}\nYour answer: {a}\n\n")).collect();
        let earlier =
            if earlier.is_empty() { String::new() } else { format!("Earlier in this conversation:\n\n{earlier}") };
        let extra = if note.extra.is_empty() { String::new() } else { format!("{}\n\n", note.extra) };
        let user = format!(
            "The code in question (lines {}-{}):\n<code>\n{}</code>\n\n{extra}{earlier}Question: {}",
            note.code_lines.start + 1,
            note.code_lines.end,
            note.code,
            note.question
        );
        let request = Prompt {
            system: "You are a concise, knowledgeable programming assistant inside a code editor. Your answer is \
                     shown as a small note right under the code it's about, so keep it short: a few sentences, \
                     specific to this code. Use a markdown code fence only when code really helps."
                .into(),
            context,
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
        if let Some(note) = &mut self.note {
            note._task = Some(task);
        }
        self.rebuild_blocks();
        cx.notify();
    }

    /// A failed test explained: the test (the item at `line`) asked about with what its run
    /// said, in a note under it.
    pub fn explain_failure(&mut self, line: usize, output: String, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.global::<Settings>().ai.ready() {
            return;
        }
        self.set_caret_point((line.min(self.buffer.len_lines().saturating_sub(1)), 0), cx);
        let lines = self.assist_lines();
        let code = self.buffer.slice(self.line_range_chars(&lines));
        let (follow_up, resized) = Self::follow_up_field(cx);
        self.note = Some(Note {
            line: lines.end,
            code_lines: lines,
            code,
            earlier: Vec::new(),
            extra: format!("The test failed. Its run said:\n<output>\n{}\n</output>", output.trim()),
            question: "Why does this test fail, and what would fix it?".into(),
            answer: String::new(),
            follow_up,
            follow_up_rows: 0,
            _follow_up_resized: resized,
            done: false,
            failed: None,
            scroll: ScrollHandle::new(),
            lines_total: self.buffer.len_lines(),
            _task: None,
        });
        self.send_note_question(cx);
        window.focus(&self.focus_handle);
        self.autoscroll = true;
    }

    /// Enter in the note's field: the next question, after the ones before.
    pub(super) fn ask_follow_up(&mut self, _: &AskFollowUp, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note) = &mut self.note else { return };
        let question = note.follow_up.read(cx).text().trim().to_string();
        if question.is_empty() || !note.done {
            return;
        }
        note.follow_up.update(cx, |input, cx| input.set_text("", cx));
        let answered = std::mem::take(&mut note.answer);
        let asked = std::mem::replace(&mut note.question, question);
        if note.failed.take().is_none() {
            note.earlier.push((asked, answered));
        }
        note.done = false;
        self.send_note_question(cx);
        window.focus(&self.focus_handle);
    }

    /// Esc in the note's field: back to the code (the note stays).
    pub(super) fn close_follow_up(&mut self, _: &CloseFollowUp, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// The answer kept as a note on the code's last line (the AI's), and the answer closed.
    pub(super) fn keep_answer_as_note(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.note.take() else { return };
        let answer = note.answer.trim();
        if answer.is_empty() {
            return;
        }
        let line = note.code_lines.end.saturating_sub(1).max(note.code_lines.start);
        self.line_notes.push(super::LineNote {
            id: crate::code_notes::new_id(),
            line,
            text: format!("{}\n{answer}", note.question.trim()),
            by_ai: true,
        });
        self.line_notes.sort_by_key(|n| n.line);
        self.line_notes_revision = self.buffer.revision();
        self.rebuild_blocks();
        cx.emit(EditorEvent::NotesChanged);
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
        let mut text = String::new();
        for (q, a) in &note.earlier {
            text.push_str(&format!("{q}\n{a}\n\n"));
        }
        text.push_str(if let Some(failed) = &note.failed { failed.as_str() } else { note.answer.as_str() });
        let text = text.as_str();
        // Text lines wrapped to the width; blank lines between paragraphs are only small gaps.
        let body: usize =
            text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.chars().count().div_ceil(per_row)).sum();
        let gaps = text.lines().filter(|l| l.trim().is_empty()).count();
        let paragraphs = text.lines().filter(|l| !l.trim().is_empty()).count();
        // As it's drawn (see the Note block): its paddings (3 + 6, above and below), the
        // question's line, 20 a line of text, 4 around each paragraph, half a line a gap;
        // in the editor's rows, rounded up so nothing is cut off.
        // (The follow-up field, once it's answered: a line and its gap.)
        let follow = if note.done { 34. } else { 0. };
        let needed = 18. + 20. + 20. * body as f32 + 4. * paragraphs as f32 + 10. * gaps as f32 + follow;
        let row = f32::from(self.line_height()).max(1.);
        // (Several lines asked: the field's rows on top, the note's text scrolls.)
        ((needed / row).ceil() as usize).clamp(2, NOTE_MAX_ROWS) + note.follow_up_rows
    }

    /// Whether some of the blocks are drawn as elements over the text, placed from its layout.
    pub fn has_overlay_blocks(&self) -> bool {
        !self.threads.is_empty()
            || self.prompt.is_some()
            || self.note.is_some()
            || !self.line_notes.is_empty()
            || self.note_draft.is_some()
    }

    #[cfg(all(test, unix))]
    pub fn threads(&self) -> &[Thread] {
        &self.threads
    }

    /// Conversations to show between the lines, each under its line.
    pub fn set_threads(&mut self, threads: Vec<Thread>, cx: &mut Context<Self>) {
        self.threads = threads;
        self.rebuild_blocks();
        cx.notify();
    }

    /// A thread's rows: as drawn (see the Thread block), rounded up so nothing's cut off.
    fn thread_rows(&self, thread: &Thread) -> usize {
        let per_row = self
            .layout
            .as_ref()
            .map(|l| ((l.text_bounds.size.width - px(60.)) / (l.char_width * 0.9)).floor().max(20.) as usize)
            .unwrap_or(80);
        let comments: f32 = thread
            .comments
            .iter()
            .map(|(_, body)| {
                let lines: usize = body.lines().map(|l| l.chars().count().div_ceil(per_row).max(1)).sum();
                18. + 20. * lines.max(1) as f32 + 8.
            })
            .sum();
        let needed = 8. + 12. + comments + 30.;
        let row = f32::from(self.line_height()).max(1.);
        ((needed / row).ceil() as usize).clamp(2, 40)
    }

    /// The blocks sized again (to the text's width now), and the rows laid out for them.
    pub fn reflow_blocks(&mut self) {
        self.rebuild_blocks();
        self.wrap.update(&self.buffer, self.wrap.width(), &self.block_specs());
    }

    pub(super) fn rebuild_blocks(&mut self) {
        self.refresh_review();
        let mut blocks = Vec::new();
        let lines = self.buffer.len_lines();
        for (i, thread) in self.threads.iter().enumerate() {
            let rows = self.thread_rows(thread);
            blocks.push(Block { before_line: (thread.line + 1).min(lines), rows, kind: BlockKind::Thread(i) });
        }
        if let Some(prompt) = &self.prompt {
            blocks.push(Block {
                before_line: prompt.lines.start,
                rows: 1 + prompt.added_rows,
                kind: BlockKind::Prompt,
            });
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
        // Notes on lines, under them (the one being changed, as its field).
        let editing = self.note_draft.as_ref().and_then(|d| d.id);
        for (i, note) in self.line_notes.iter().enumerate() {
            if Some(note.id) != editing {
                let rows = Self::line_note_rows(note);
                blocks.push(Block { before_line: (note.line + 1).min(lines), rows, kind: BlockKind::LineNote(i) });
            }
        }
        if let Some(draft) = &self.note_draft {
            blocks.push(Block { before_line: (draft.line + 1).min(lines), rows: 1, kind: BlockKind::NoteDraft });
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
            // (Text only looked at, as a commit has it: nothing to keep or undo.)
            if let Some(index) = self.current_hunk().filter(|_| !self.view_only) {
                let hunk = &review.hunks[index];
                blocks.push(Block {
                    before_line: hunk.new.end.max(hunk.new.start),
                    rows: 1,
                    kind: BlockKind::ReviewHint(review.hunks.len() - 1, review.of_ai),
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

    /// The keys on the "Writing…" bar once it's done writing (when it's drawn, so also when
    /// they come back to it later, as a list closing gives them back): to the field if it's
    /// still there (the answer didn't come: ask again), else to the code (written: ⇥ and Esc
    /// keep or undo it). Keys anywhere else are left there.
    pub(super) fn keys_back_after_writing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !window.focused(cx).is_some_and(|f| f == self.writing_focus) {
            return;
        }
        match &self.prompt {
            Some(prompt) if prompt.writing => {}
            Some(prompt) => window.focus(&prompt.input.focus_handle(cx)),
            None => window.focus(&self.focus_handle),
        }
    }

    /// Whether the keys are this editor's: in its text, or on its "Writing…" bar.
    pub fn has_keys(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.contains_focused(window, cx) || self.writing_focus.is_focused(window)
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
                        (Some("Writing…".into()), "Esc cancel")
                    } else if let Some(failed) = &prompt.failed {
                        (None, if failed.len() > 60 { "" } else { failed.as_str() })
                    } else {
                        let typed = prompt.input.read(cx).text().trim().to_string();
                        let hint = if typed.is_empty() {
                            if prompt.error.is_some() { "↵ fix error" } else { "⇧↵ new line" }
                        } else if prompt.ask_only || is_question(&typed) {
                            "↵ ask"
                        } else {
                            "↵ change"
                        };
                        (None, hint)
                    };
                    let failed_long = prompt.failed.as_ref().filter(|f| f.len() > 60).cloned();
                    // Several lines typed: what's beside the field stays by its first.
                    let tall = block.rows > 1;
                    // What goes with it, while nothing's wrong and nothing's being written.
                    let sends = (!prompt.writing && prompt.failed.is_none()).then(|| prompt.sends.clone());
                    out.push(
                        place(div())
                            // While it writes, the keys are here (see `writing_focus`).
                            .when(prompt.writing, |d| d.track_focus(&self.writing_focus))
                            // A click on it stays on it (not, through it, in the code below).
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .key_context("AiPrompt")
                            .on_action(cx.listener(Self::submit_prompt))
                            .on_action(cx.listener(Self::cancel_prompt))
                            .flex()
                            .when(tall, |d| d.items_start().pt(px(4.)))
                            .when(!tall, |d| d.items_center())
                            .gap(px(10.))
                            .px(px(10.))
                            .rounded(px(ui::R_CONTROL))
                            .border_1()
                            .border_color(if prompt.writing { theme.caret.opacity(0.5) } else { theme.hairline })
                            .bg(theme.raised)
                            .text_size(px(ui::T_MD))
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
                            .children(sends.map(|sends| {
                                div()
                                    .flex_none()
                                    .max_w(px(420.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_size(px(ui::T_XS))
                                    .text_color(theme.faint)
                                    .child(sends)
                            }))
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
                                    .child(ui::key_words(hint)),
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
                BlockKind::ReviewHint(more, of_ai) => {
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
                            .child(key(&super::UndoHunk, "Esc", if *of_ai { "undo" } else { "discard" }))
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
                    let answer = |text: &str| -> Vec<AnyElement> {
                        crate::markdown::blocks(text, None)
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
                    let asked = |q: &str| {
                        div()
                            .pt(px(6.))
                            .text_size(px(ui::T_XS))
                            .text_color(theme.muted)
                            .child(q.to_string())
                            .into_any_element()
                    };
                    // The questions before, each with its answer; then this one's.
                    let mut body: Vec<AnyElement> = Vec::new();
                    for (i, (q, a)) in note.earlier.iter().enumerate() {
                        if i > 0 {
                            body.push(asked(q));
                        }
                        body.extend(answer(a));
                    }
                    if !note.earlier.is_empty() {
                        body.push(asked(&note.question));
                    }
                    if let Some(failed) = &note.failed {
                        body.push(div().text_color(theme.error).child(failed.clone()).into_any_element());
                    } else if note.answer.trim().is_empty() {
                        body.push(div().text_color(theme.muted).child("Thinking…").into_any_element());
                    } else {
                        body.extend(answer(&note.answer));
                    }
                    let first = note.earlier.first().map_or(note.question.clone(), |(q, _)| q.clone());
                    // Once answered: a follow-up, or the answer kept as a note on the code.
                    let follow = note.done.then(|| {
                        div()
                            .mt(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .key_context("NoteFollowUp")
                            .on_action(cx.listener(Self::ask_follow_up))
                            .on_action(cx.listener(Self::close_follow_up))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .px(px(8.))
                                    .py(px(3.))
                                    .rounded(px(ui::R_CONTROL))
                                    .border_1()
                                    .border_color(theme.hairline)
                                    .child(note.follow_up.clone()),
                            )
                            .when(note.failed.is_none(), |d| {
                                d.child(
                                    div()
                                        .id("keep-answer")
                                        .flex_none()
                                        .text_size(px(ui::T_XS))
                                        .text_color(theme.muted)
                                        .cursor_pointer()
                                        .hover(|s| s.text_color(theme.foreground))
                                        .child("Keep as note")
                                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                            this.keep_answer_as_note(cx)
                                        })),
                                )
                            })
                    });
                    out.push(
                        place(div())
                            .py(px(3.))
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
                                    .text_size(px(ui::T_MD))
                                    .line_height(px(20.))
                                    .text_color(theme.foreground)
                                    .child(
                                        div()
                                            .flex()
                                            .justify_between()
                                            .text_size(px(ui::T_XS))
                                            .text_color(theme.muted)
                                            .child(div().overflow_hidden().whitespace_nowrap().child(first))
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .pl(px(12.))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(if note.done { "Esc close" } else { "" }),
                                            ),
                                    )
                                    .children(body)
                                    .children(follow),
                            )
                            .into_any_element(),
                    );
                }
                BlockKind::Thread(i) => {
                    let Some(thread) = self.threads.get(*i) else { continue };
                    let id = thread.id;
                    let comments = thread.comments.iter().map(|(who, body)| {
                        div()
                            .pb(px(8.))
                            .child(
                                div()
                                    .text_size(px(ui::T_XS))
                                    .line_height(px(18.))
                                    .text_color(theme.muted)
                                    .child(who.clone()),
                            )
                            .children(body.lines().map(|l| div().min_h(px(20.)).child(l.to_string())))
                    });
                    out.push(
                        place(div())
                            .py(px(4.))
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .size_full()
                                    .overflow_hidden()
                                    .rounded(px(ui::R_CONTROL))
                                    .border_1()
                                    .border_color(theme.hairline)
                                    .bg(theme.surface)
                                    .px(px(12.))
                                    .py(px(6.))
                                    .font_family(cx.global::<crate::fonts::Fonts>().ui.clone())
                                    .text_size(px(ui::T_MD))
                                    .line_height(px(20.))
                                    .text_color(theme.foreground)
                                    .children(comments)
                                    .child(
                                        div()
                                            .id(("thread-reply", *i))
                                            .px(px(8.))
                                            .py(px(3.))
                                            .rounded(px(ui::R_KEY))
                                            .border_1()
                                            .border_color(theme.hairline)
                                            .text_color(theme.faint)
                                            .cursor_pointer()
                                            .hover(|s| s.text_color(theme.muted))
                                            .child("Reply…")
                                            .on_click(cx.listener(move |_, _: &gpui::ClickEvent, _, cx| {
                                                cx.emit(EditorEvent::ReplyAsked(id))
                                            })),
                                    ),
                            )
                            .into_any_element(),
                    );
                }
                BlockKind::LineNote(i) => {
                    let Some(note) = self.line_notes.get(*i) else { continue };
                    let (id, by_ai) = (note.id, note.by_ai);
                    out.push(
                        place(div())
                            .py(px(2.))
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .id(("line-note", id as usize))
                                    .group("line-note")
                                    .size_full()
                                    .overflow_hidden()
                                    .flex()
                                    .gap(px(10.))
                                    .border_l_2()
                                    .border_color(if by_ai { theme.caret.opacity(0.7) } else { theme.faint })
                                    .pl(px(10.))
                                    .pr(px(6.))
                                    .font_family(cx.global::<crate::fonts::Fonts>().ui.clone())
                                    .text_size(px(ui::T_MD))
                                    .line_height(px(20.))
                                    .text_color(theme.muted)
                                    .cursor_pointer()
                                    .hover(|s| s.text_color(theme.foreground))
                                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                                        this.edit_note(id, window, cx)
                                    }))
                                    .when(by_ai, |d| {
                                        d.child(
                                            div()
                                                .flex_none()
                                                .text_size(px(ui::T_XS))
                                                .text_color(theme.caret)
                                                .child("AI"),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .children(note.text.lines().map(|l| div().child(l.to_string()))),
                                    )
                                    .child(
                                        div()
                                            .id(("line-note-remove", id as usize))
                                            .flex_none()
                                            .text_size(px(ui::T_XS))
                                            .text_color(theme.faint)
                                            .invisible()
                                            .group_hover("line-note", |s| s.visible())
                                            .hover(|s| s.text_color(theme.foreground))
                                            .child("✕")
                                            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                                                cx.stop_propagation();
                                                this.remove_note(id, cx)
                                            })),
                                    ),
                            )
                            .into_any_element(),
                    );
                }
                BlockKind::NoteDraft => {
                    let Some(draft) = &self.note_draft else { continue };
                    let hint = if draft.input.read(cx).text().trim().is_empty() && draft.id.is_some() {
                        "↵ remove · Esc"
                    } else {
                        "↵ keep · Esc"
                    };
                    out.push(
                        place(div())
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .key_context("NoteDraft")
                            .on_action(cx.listener(Self::keep_note))
                            .on_action(cx.listener(Self::leave_note))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .border_l_2()
                            .border_color(theme.muted)
                            .pl(px(10.))
                            .pr(px(10.))
                            .bg(theme.surface)
                            .text_size(px(ui::T_MD))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_color(theme.foreground)
                                    .child(draft.input.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(ui::T_XS))
                                    .text_color(theme.muted)
                                    .child(ui::key_words(hint)),
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

/// `answer` without the lines at its start that repeat the end of `before` (the file above
/// the part asked about), and those at its end that repeat the start of `after`: something
/// of the answer stays.
fn without_context(answer: &str, before: &[String], after: &[String], asked: usize) -> String {
    let lines: Vec<&str> = answer.lines().collect();
    if lines.is_empty() {
        return answer.to_string();
    }
    let same = |a: &str, b: &str| a.trim_end() == b.trim_end();
    let most = lines.len().saturating_sub(1);
    let head = (1..=most.min(before.len()))
        .rev()
        .find(|&k| lines[..k].iter().zip(&before[before.len() - k..]).all(|(a, b)| same(a, b)))
        .unwrap_or(0);
    let room = lines.len() - head - 1;
    let tail = (1..=room.min(after.len()))
        .rev()
        .find(|&m| lines[lines.len() - m..].iter().zip(&after[..m]).all(|(a, b)| same(a, b)))
        .unwrap_or(0);
    // Only when it's surely what's around: more than one line of it, with the answer at
    // least as long as the part asked about besides (`x += 1` above and below a line
    // asked about isn't taken for them).
    if head + tail < 2 || lines.len() < asked + head + tail {
        return answer.to_string();
    }
    let mut kept = lines[head..lines.len() - tail].join("\n");
    if answer.ends_with('\n') {
        kept.push('\n');
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The function around the lines asked about (or the whole file) sent back: only the
    /// part asked about is kept.
    #[test]
    fn code_around_the_part_asked_about_is_left_out() {
        let lines = |text: &str| text.lines().map(String::from).collect::<Vec<_>>();
        let before = lines("fn main() {");
        let after = lines("    let unused = 3;\n}\n\nfn helper() -> i32 {\n    42\n}");
        let answer = "fn main() {\n    let first = 5;\n    let second = 2;\n    println!(\"{}\", first + second);\n    let unused = 3;\n}\n\nfn helper() -> i32 {\n    42\n}\n";
        assert_eq!(
            without_context(answer, &before, &after, 3),
            "    let first = 5;\n    let second = 2;\n    println!(\"{}\", first + second);\n"
        );
        // Just the part: as it is.
        let part = "    let first = 5;\n";
        assert_eq!(without_context(part, &before, &after, 1), part);
        // An answer that is only the lines around: one line of it stays (not nothing).
        assert_eq!(without_context("fn main() {\n", &before, &after, 1), "fn main() {\n");
        // Nothing: nothing (not a crash).
        assert_eq!(without_context("", &before, &after, 1), "");
        // A line like the ones around, written by the change itself: kept.
        let around = lines("x += 1");
        assert_eq!(without_context("# bump\nx += 1", &around, &around, 1), "# bump\nx += 1");
    }
    use crate::buffer::Buffer;
    use gpui::{EntityInputHandler as _, TestAppContext, VisualTestContext};
    use std::path::PathBuf;

    fn editor<'a>(cx: &'a mut TestAppContext, text: &str) -> (Entity<Editor>, &'a mut VisualTestContext) {
        cx.update(|cx| {
            // (A provider chosen: ⌘I opens only when AI can answer.)
            let mut settings = Settings::default();
            settings.ai.provider = crate::ai::ProviderId::Ollama;
            settings.ai.enabled = true;
            cx.set_global(settings);
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

    /// ⌘I again on a change adjusts it: the code goes back as it was, and the field is on
    /// the lines first asked about (not as many as the change made, which would reach the
    /// code below).
    #[gpui::test]
    fn adjusting_a_change_asks_about_the_lines_it_replaced(cx: &mut TestAppContext) {
        let text = "def f(x):\n    return x\n\ndef g():\n    pass\n";
        let (e, cx) = editor(cx, text);
        e.update_in(cx, |e, window, cx| {
            e.selection = Selection::caret(2);
            e.open_inline_assist(false, window, cx);
            assert_eq!(e.assist_target(), Some(0..2));
            e.apply_change("def f(x):\n    y = x\n    z = y\n    return z\n".into(), "longer".into(), cx);
            e.open_inline_assist(false, window, cx);
            assert_eq!(e.buffer.to_string(), text);
            assert_eq!(e.assist_target(), Some(0..2));
        });
    }

    /// Where the keys are while ⌘I writes a change and after (the 30th and 31st reviews'
    /// rules): on the "Writing…" bar while it writes, where Esc stops it and typing reaches
    /// nothing; in the code once written, even when something else had them meanwhile and
    /// gave them back to the bar (a list closing); Esc from the code stops it too.
    #[gpui::test]
    fn the_keys_while_a_change_is_written_and_after(cx: &mut TestAppContext) {
        let text = "def f(x):\n    return x / 0\n";
        let (e, cx) = editor(cx, text);
        cx.update(|_, cx| crate::keymap::register(crate::keymap::Keymap::Null, cx));
        // Writing, as ↵ in the field leaves it: the bar has the keys.
        let writing = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.update_in(cx, |e, window, cx| {
                e.selection = Selection::caret(2);
                e.open_inline_assist(false, window, cx);
                if let Some(prompt) = &mut e.prompt {
                    prompt.writing = true;
                }
                window.focus(&e.writing_focus);
                e.rebuild_blocks();
                cx.notify();
            });
            cx.run_until_parked();
            // (The bar is drawn where the last frame laid the text out: a second frame.)
            e.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
        };
        let keys_on = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.update_in(cx, |e, window, _| (e.writing_focus.is_focused(window), e.focus_handle.is_focused(window)))
        };
        writing(&e, cx);
        assert_eq!(keys_on(&e, cx), (true, false), "while it writes: the bar's");
        // Typed then: nothing reaches the code. Esc: it stops.
        cx.simulate_keystrokes("x y z enter");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), text);
        cx.simulate_keystrokes("escape");
        assert!(e.read_with(cx, |e, _| e.prompt.is_none()), "Esc on the bar stops it");

        // Written: the keys go to the code, to keep or undo it.
        writing(&e, cx);
        e.update(cx, |e, cx| e.apply_change("def f(x):\n    return x / 2\n".into(), "halve".into(), cx));
        cx.run_until_parked();
        assert_eq!(keys_on(&e, cx), (false, true), "written: the code's");
        cx.simulate_keystrokes("tab");
        assert!(e.read_with(cx, |e, _| e.ai_change.is_none()), "⇥ keeps it");

        // A list had the keys when it was written, and gives them back to the bar on closing.
        writing(&e, cx);
        let elsewhere = cx.update(|_, cx| cx.focus_handle());
        cx.update(|window, _| window.focus(&elsewhere));
        e.update(cx, |e, cx| e.apply_change("def f(x):\n    return x / 4\n".into(), "quarter".into(), cx));
        cx.run_until_parked();
        assert!(cx.update(|window, _| elsewhere.is_focused(window)), "not taken from the list");
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.writing_focus);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(keys_on(&e, cx), (false, true), "given back to the bar, gone: the code's");
        cx.simulate_keystrokes("escape");
        assert!(e.read_with(cx, |e, _| e.ai_change.is_none()), "Esc undoes it");

        // The keys in the code while it writes (a click there): Esc still stops it.
        writing(&e, cx);
        e.update_in(cx, |e, window, _| window.focus(&e.focus_handle));
        cx.simulate_keystrokes("escape");
        assert!(e.read_with(cx, |e, _| e.prompt.is_none()), "Esc in the code stops it");
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
            // (AI on, as suggestions while typing need, but no provider chosen.)
            settings.ai.enabled = true;
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
