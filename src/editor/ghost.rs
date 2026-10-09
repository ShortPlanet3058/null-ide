//! Suggestions while typing, shown faintly after the caret; ⇥ takes them.
//!
//! Two sources, fastest first:
//! - names from the file: typing `pro` next to a `process_records` defined above
//!   suggests the rest at once, no AI involved;
//! - the AI, after a short pause, which can go further (the arguments, the rest of
//!   the line) and streams in as it arrives.
//!
//! Typing the characters a suggestion shows keeps it, so it never flickers away.
//! ⇥ takes it all, ⌥→ the next word, ⌘→ the rest of the line; ⌥⇥ shows another option.
//! Off unless switched on in Settings → AI.

use super::{EditKind, Editor};
use crate::ai::{self, AiEvent, Prompt, ProviderId};
use crate::settings::Settings;
use futures::StreamExt;
use gpui::{App, Context, KeyBinding, Window, actions};
use std::collections::HashMap;
use std::time::Duration;

actions!(ghost, [AcceptGhost, AcceptGhostWord, AcceptGhostLine, NextGhost, PreviousGhost, DismissGhost]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Editor && ai_ghost");
    cx.bind_keys([
        KeyBinding::new("tab", AcceptGhost, ctx),
        KeyBinding::new("alt-right", AcceptGhostWord, ctx),
        KeyBinding::new("secondary-right", AcceptGhostLine, ctx),
        // ⌥⇥ rather than ⌥] : brackets take several keys on many layouts.
        KeyBinding::new("alt-tab", NextGhost, ctx),
        KeyBinding::new("alt-shift-tab", PreviousGhost, ctx),
        KeyBinding::new("escape", DismissGhost, ctx),
    ]);
}

/// Wait this long after the last keystroke before asking the AI.
const PAUSE: Duration = Duration::from_millis(250);
const MAX_LINES: usize = 6;
/// The file sent to the AI is cut to about this many characters around the caret.
const CONTEXT_BEFORE: usize = 12_000;
const CONTEXT_AFTER: usize = 3_000;
/// Files the current one includes or imports, given to the AI: at most this many, cut to this size.
const RELATED_FILES: usize = 3;
const RELATED_CHARS: usize = 3_000;
/// Other open tabs, cut to this size.
const OPEN_FILE_CHARS: usize = 2_000;
/// Suggestions remembered by where they were made, so deleting back brings one at once.
const CACHE_SIZE: usize = 60;
/// Names suggested from the file need at least this much typed.
const MIN_PREFIX: usize = 2;
/// How far around the caret names are looked for, in lines and characters.
const NAMES_AROUND: usize = 2000;
const NAMES_AROUND_CHARS: usize = 100_000;

pub(super) struct Ghost {
    /// Where it goes, and the text it was made for: it disappears once either changes.
    pub(super) offset: usize,
    pub(super) version: u64,
    pub(super) lines: Vec<String>,
    /// The options seen so far here (⌥⇥ goes through them, then asks for more).
    alternatives: Vec<Vec<String>>,
    current: usize,
    /// How to ask for another option.
    request: Option<std::rc::Rc<SuggestionRequest>>,
}

impl Ghost {
    pub(super) fn new(offset: usize, version: u64, lines: Vec<String>) -> Self {
        Self { offset, version, alternatives: vec![lines.clone()], lines, current: 0, request: None }
    }
}

/// A question for the AI about one spot, kept to ask again for another option.
pub(super) struct SuggestionRequest {
    offset: usize,
    version: u64,
    /// The current line before and after the caret, and the text after it.
    before: String,
    after: String,
    suffix: String,
    block: bool,
    single_line: bool,
    use_fill: bool,
    fill: ai::Fim,
    chat: Prompt,
    ai_settings: ai::AiSettings,
    cache_key: String,
}

/// The start of a suggestion up to the end of its next word (or its next line break,
/// with the indentation after it).
fn next_word(text: &str) -> &str {
    if let Some(rest) = text.strip_prefix('\n') {
        let indent = rest.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        return &text[..1 + indent];
    }
    let mut end = 0;
    let mut chars = text.char_indices().peekable();
    while let Some(&(i, c)) = chars.peek() {
        if c == ' ' || c == '\t' {
            end = i + c.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    let Some(&(start, first)) = chars.peek() else { return &text[..end] };
    let word = is_ident(first);
    for (i, c) in chars {
        if c.is_whitespace() || is_ident(c) != word || (!word && i > start) {
            break;
        }
        end = i + c.len_utf8();
    }
    &text[..end]
}

/// The start of a suggestion up to the end of its line (or the whole next line, when
/// it starts with a line break).
fn next_line(text: &str) -> &str {
    let start = usize::from(text.starts_with('\n'));
    match text[start..].find('\n') {
        Some(i) => &text[..start + i],
        None => text,
    }
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The name being typed: the identifier characters just before the caret.
fn typed_word(before: &str) -> &str {
    let start = before.char_indices().rev().take_while(|(_, c)| is_ident(*c)).last().map_or(before.len(), |(i, _)| i);
    &before[start..]
}

/// The best name in `text` starting with `prefix`: names defined in the file
/// (`def`, `fn`, `class`…) first, then the most used, then the nearest to `near`.
fn best_name(text: &str, prefix: &str, near: usize) -> Option<String> {
    let mut found: HashMap<&str, (usize, usize, bool)> = HashMap::new();
    let mut start = None;
    let mut previous_word = "";
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        match (start, is_ident(c)) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                let word = &text[s..i];
                if word.len() > prefix.len()
                    && word.starts_with(prefix)
                    && !word.starts_with(|c: char| c.is_ascii_digit())
                {
                    let defined = matches!(
                        previous_word,
                        "def"
                            | "fn"
                            | "function"
                            | "class"
                            | "struct"
                            | "enum"
                            | "let"
                            | "const"
                            | "var"
                            | "func"
                            | "type"
                            | "trait"
                            | "interface"
                    );
                    let entry = found.entry(word).or_insert((0, usize::MAX, false));
                    entry.0 += 1;
                    entry.1 = entry.1.min(s.abs_diff(near));
                    entry.2 |= defined;
                }
                previous_word = word;
                start = None;
            }
            _ => {}
        }
    }
    found
        .into_iter()
        .max_by_key(|(_, (count, distance, defined))| (*defined, *count, std::cmp::Reverse(*distance)))
        .map(|(word, _)| word.to_string())
}

/// Models whose fill requests failed this session: suggestions use chat for them instead.
static FILL_FAILED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn fill_key(settings: &ai::AiSettings) -> String {
    let id = settings.active();
    format!("{}:{}", id.key(), settings.completion_model(id).unwrap_or_default())
}

fn fill_failed(settings: &ai::AiSettings) -> bool {
    FILL_FAILED.lock().is_ok_and(|f| f.contains(&fill_key(settings)))
}

fn mark_fill_failed(settings: &ai::AiSettings) {
    if let Ok(mut failed) = FILL_FAILED.lock() {
        failed.push(fill_key(settings));
    }
}

/// The suggestion's lines: at most a few, no trailing blank ones, and stopping where
/// it starts repeating the code that already follows the caret.
fn shape_suggestion(completion: &str, suffix: &str, block: bool) -> Vec<String> {
    let following: Vec<&str> = suffix.lines().skip(1).map(str::trim).filter(|l| !l.is_empty()).take(2).collect();
    let all: Vec<&str> = completion.lines().collect();
    let repeats_from = |i: usize| {
        let rest: Vec<&str> = all[i..].iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
        match (following.as_slice(), rest.as_slice()) {
            ([first, second], [a, b, ..]) => a == first && b == second,
            ([first, ..], [a]) => a == first,
            ([first], [a, ..]) => a == first && first.len() > 2,
            _ => false,
        }
    };
    let mut lines: Vec<String> = Vec::new();
    for (i, line) in all.iter().enumerate() {
        if i > 0 && repeats_from(i) {
            break;
        }
        lines.push(line.trim_end().to_string());
        if lines.len() >= if block { MAX_LINES + 2 } else { MAX_LINES } {
            break;
        }
    }
    while lines.len() > 1 && lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines
}

/// Removes what the AI repeated of the line before the caret, and what's already after it.
fn trim_overlap(before: &str, after: &str, completion: &str) -> String {
    let mut text = completion.to_string();
    // A model that "restarts" the line: drop the part already typed.
    let max = before.len().min(text.len());
    if let Some(k) = (1..=max).rev().find(|&k| {
        text.is_char_boundary(k) && before.is_char_boundary(before.len() - k) && before.ends_with(&text[..k])
    }) {
        // Only a real overlap: at least a word's worth, or the whole typed name.
        if k >= 3 || k == typed_word(before).len() {
            text = text[k..].to_string();
        }
    }
    // Brackets already closed after the caret: drop them from the end of the
    // suggestion when keeping them would close one too many.
    let after = after.trim_end();
    let open = |t: &str| t.chars().filter(|c| matches!(c, '(' | '[' | '{')).count() as isize;
    let close = |t: &str| t.chars().filter(|c| matches!(c, ')' | ']' | '}')).count() as isize;
    if !after.is_empty()
        && let Some(first) = text.lines().next()
        && first.trim_end().ends_with(after)
        && open(before) + open(first) - close(before) - close(first) < close(after)
    {
        let cut = first.trim_end().len() - after.len();
        text = format!("{}{}", &first[..cut], &text[first.len()..]);
    }
    text
}

impl Editor {
    /// The ghost's first line (drawn after the caret) and the lines below it.
    pub fn ghost_text(&self) -> Option<(&str, &[String])> {
        let ghost = self.ghost.as_ref()?;
        let (first, rest) = ghost.lines.split_first()?;
        Some((first.as_str(), rest))
    }

    pub fn ghost_line(&self) -> Option<usize> {
        self.ghost.as_ref().map(|g| self.buffer.point(g.offset).0)
    }

    /// Drops a ghost that no longer matches the caret or the text.
    pub(super) fn check_ghost(&mut self) {
        if self.ghost.as_ref().is_some_and(|g| g.offset != self.selection.head || g.version != self.buffer.version()) {
            self.ghost = None;
            self.rebuild_blocks();
        }
    }

    fn set_ghost(&mut self, lines: Vec<String>, cx: &mut Context<Self>) {
        self.ghost = (!lines.iter().all(|l| l.is_empty()))
            .then(|| Ghost::new(self.selection.head, self.buffer.version(), lines));
        self.rebuild_blocks();
        cx.notify();
    }

    /// Called just before typing `text`: if it's what the suggestion shows next, the
    /// suggestion stays, minus those characters. Returns whether it did.
    pub(super) fn type_into_ghost(&mut self, text: &str) -> Option<Ghost> {
        let ghost = self.ghost.take()?;
        let first = ghost.lines.first()?;
        (ghost.offset == self.selection.head
            && self.selection.is_empty()
            && first.starts_with(text)
            && !text.is_empty())
        .then(|| {
            let mut lines = ghost.lines.clone();
            lines[0] = first[text.len()..].to_string();
            Ghost::new(ghost.offset + text.chars().count(), 0, lines)
        })
    }

    /// After typing: suggest at once from the file (or from what was suggested here
    /// before), then ask the AI after a pause. `kept` is a suggestion that what was
    /// typed matched, which then stays as it is.
    /// The name from the file `prefix` most likely starts, looked for around the caret
    /// (2000 lines, 100k characters each way): typing stays quick in a big file.
    fn best_name_near(&self, prefix: &str) -> Option<String> {
        let caret = self.selection.head;
        let line = self.buffer.point(caret).0;
        let from =
            self.buffer.line_to_char(line.saturating_sub(NAMES_AROUND)).max(caret.saturating_sub(NAMES_AROUND_CHARS));
        let to = self.buffer.line_to_char(line + NAMES_AROUND).min(caret + NAMES_AROUND_CHARS);
        let text = self.buffer.slice(from..to);
        let rope = self.buffer.rope();
        best_name(&text, prefix, rope.char_to_byte(caret) - rope.char_to_byte(from))
    }

    pub(super) fn schedule_ghost(&mut self, kept: Option<Ghost>, cx: &mut Context<Self>) {
        self.ghost_task = None;
        let ai_settings = cx.global::<Settings>().ai.clone();
        if !ai_settings.completions {
            return;
        }
        if !self.selection.is_empty() || self.multi_cursor() || self.completion.is_some() || self.prompt.is_some() {
            return;
        }
        if let Some(mut ghost) = kept.filter(|g| g.lines.iter().any(|l| !l.is_empty())) {
            ghost.version = self.buffer.version();
            self.ghost = Some(ghost);
            self.rebuild_blocks();
            return cx.notify();
        }
        let (line, col) = self.buffer.point(self.selection.head);
        let line_text = self.buffer.line_text(line);
        let split = line_text.char_indices().nth(col).map_or(line_text.len(), |(b, _)| b);
        let (before, after) = (line_text[..split].to_string(), line_text[split..].to_string());
        // Not in the middle of a word.
        if after.chars().next().is_some_and(is_ident) {
            return;
        }
        // Mid-line (more than closing brackets after the caret): one line, from a model that sees both sides.
        let mid_line =
            !after.chars().all(|c| c.is_whitespace() || matches!(c, ')' | ']' | '}' | ';' | ',' | '"' | '\'' | ':'));

        // 1. Something known right away: what was suggested here before, or a name from the file.
        let key = self.ghost_cache_key();
        if let Some(lines) = self.ghost_cache.iter().rev().find(|(k, _)| *k == key).map(|(_, l)| l.clone()) {
            return self.set_ghost(lines, cx);
        }
        let word = typed_word(&before);
        if word.chars().count() >= MIN_PREFIX
            && let Some(name) = self.best_name_near(word)
        {
            self.set_ghost(vec![name[word.len()..].to_string()], cx);
        } else if self.ghost.is_some() {
            self.ghost = None;
            self.rebuild_blocks();
        }

        // 2. The AI, after a pause. The command line tools take seconds to start: too slow for this.
        let provider = ai_settings.active();
        if matches!(provider, ProviderId::Off | ProviderId::ClaudeCode | ProviderId::Codex) {
            return;
        }
        let use_fill = ai::fim_available(&ai_settings) && !fill_failed(&ai_settings);
        if mid_line && !use_fill {
            return;
        }
        // What's sent (the file around, the project's names) put together once typing
        // pauses, not on every keystroke.
        let (offset, version) = (self.selection.head, self.buffer.version());
        self.ghost_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE).await;
            this.update(cx, |this, cx| {
                if this.selection.head != offset || this.buffer.version() != version {
                    return;
                }
                let request =
                    std::rc::Rc::new(this.suggestion_request(ai_settings, use_fill, before, after, mid_line, key, cx));
                this.request_suggestion(request, false, true, cx);
            })
            .ok();
        }));
    }

    /// The text before and after the caret, cut to size, for the cache.
    fn ghost_cache_key(&self) -> String {
        let offset = self.selection.head;
        let start = offset.saturating_sub(400);
        let end = (offset + 120).min(self.buffer.len_chars());
        format!("{}\u{0}{}", self.buffer.slice(start..offset), self.buffer.slice(offset..end))
    }

    /// Everything the AI gets: the file around the caret, what the project defines, the
    /// files this one includes or imports, and the other open tabs.
    #[allow(clippy::too_many_arguments)]
    fn suggestion_request(
        &self,
        ai_settings: ai::AiSettings,
        use_fill: bool,
        before: String,
        after: String,
        single_line: bool,
        cache_key: String,
        cx: &App,
    ) -> SuggestionRequest {
        let provider = ai_settings.active();
        let offset = self.selection.head;
        // Only the text around the caret: the file can be big.
        let rope = self.buffer.rope();
        let caret_byte = rope.char_to_byte(offset);
        let from_byte = caret_byte.saturating_sub(CONTEXT_BEFORE);
        let mut from = rope.byte_to_char(from_byte);
        if rope.char_to_byte(from) < from_byte {
            from += 1;
        }
        let to = rope.byte_to_char((caret_byte + CONTEXT_AFTER).min(rope.len_bytes()));
        let prefix = self.buffer.slice(from..offset);
        let path_buf = self.path.clone().unwrap_or_default();
        let path = self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "untitled".into());
        let comment = self.language().and_then(|l| l.line_comment).unwrap_or("//");

        let related = self.related_files();
        let mut others: Vec<(String, String)> = related.clone();
        let mut outline = String::new();
        if let Some(project) = cx.try_global::<crate::project_index::ProjectContext>() {
            outline = crate::project_index::outline_for(&project.definitions, &path_buf, &project.root, comment);
            for (open, body) in &project.open_files {
                let name = open.strip_prefix(&project.root).unwrap_or(open).display().to_string();
                if *open != path_buf && !others.iter().any(|(n, _)| *n == name) {
                    others.push((name, body.chars().take(OPEN_FILE_CHARS).collect()));
                }
            }
        }
        let suffix = self.buffer.slice(offset..to);
        let files: String = others.iter().map(|(name, body)| format!("{comment} File: {name}\n{body}\n\n")).collect();
        let outline_block = if outline.is_empty() {
            String::new()
        } else {
            format!("{comment} Defined elsewhere in the project:\n{outline}\n")
        };
        let fill = ai::Fim {
            prefix: format!("{outline_block}{files}{comment} File: {path}\n{prefix}"),
            suffix: suffix.clone(),
            max_tokens: if single_line { 48 } else { 160 },
            temperature: 0.2,
        };
        let chat_context: String = others
            .iter()
            .map(|(name, body)| format!("Another file of the project, {name}:\n<file>\n{body}\n</file>\n\n"))
            .collect();
        let chat_outline = if outline.is_empty() {
            String::new()
        } else {
            format!("Defined elsewhere in the project:\n{outline}\n\n")
        };
        let chat = Prompt {
            system: format!(
                "You are the code completion in a code editor. The person is typing at <CURSOR>. Reply with \
                 only the characters that come next, continuing exactly from the cursor, as if you were \
                 typing them. Never repeat anything that is already before the cursor, not even part of the \
                 current word. Finish the line, and when the code clearly goes on (a function body, the rest \
                 of a block) write up to {MAX_LINES} lines, ending at a natural point. Use names from the \
                 project, and well-known idioms. No explanation, no markdown, no fences. If nothing obvious \
                 comes next, reply with nothing."
            ),
            user: format!(
                "{chat_outline}{chat_context}File: {path} ({})\n\n{}<CURSOR>{}\n\nThe current line, up to the cursor: {before:?}",
                self.language_name(),
                prefix,
                suffix,
            ),
            // A fill-only code model can't chat; the usual model does it then.
            model: ai_settings.completion_model(provider).filter(|_| !ai::fim_available(&ai_settings)),
            max_tokens: Some(160),
            // Thinking makes suggestions arrive too late to help: the least the provider allows.
            effort: provider.efforts().iter().copied().find(|e| *e != ai::Effort::Auto),
            temperature: None,
        };
        let block = before.trim().is_empty() || before.trim_end().ends_with([':', '{', '(', '[']);
        SuggestionRequest {
            offset,
            version: self.buffer.version(),
            before,
            after,
            suffix,
            block,
            single_line,
            use_fill,
            fill,
            chat,
            ai_settings,
            cache_key,
        }
    }

    /// Asks the AI. As a first suggestion it waits for the typing to pause and shows the
    /// answer as it streams; as another option (⌥⇥) it asks for something different and
    /// adds it to the ones already seen.
    /// Asks for a suggestion: `another` one (warmer), or after typing pauses (unless that
    /// was `waited` for already).
    fn request_suggestion(
        &mut self,
        request: std::rc::Rc<SuggestionRequest>,
        another: bool,
        waited: bool,
        cx: &mut Context<Self>,
    ) {
        self.ghost_task = Some(cx.spawn(async move |this, cx| {
            if !another && !waited {
                cx.background_executor().timer(PAUSE).await;
            }
            let (offset, version) = (request.offset, request.version);
            let still_there = this
                .update(cx, |this, _| this.selection.head == offset && this.buffer.version() == version)
                .unwrap_or(false);
            if !still_there {
                return;
            }
            let started = std::time::Instant::now();
            let settings = request.ai_settings.clone();
            let mut events = if request.use_fill {
                let mut fill = request.fill.clone();
                if another {
                    fill.temperature = 0.8;
                }
                ai::stream_fim(settings.clone(), fill)
            } else {
                let mut chat = request.chat.clone();
                if another {
                    chat.temperature = Some(0.8);
                }
                ai::stream(settings.clone(), chat)
            };
            let mut answer = String::new();
            while let Some(event) = events.next().await {
                let done = match event {
                    AiEvent::Text(chunk) => {
                        answer.push_str(&chunk);
                        false
                    }
                    AiEvent::Done => true,
                    AiEvent::Failed(message) => {
                        if request.use_fill {
                            // That model or endpoint can't fill: use the chat model from now on.
                            mark_fill_failed(&settings);
                        } else {
                            ai::log_request(&settings, "chat", started, None, &Err(message));
                        }
                        return;
                    }
                };
                // Show it as it comes, a line at a time (another option only once complete).
                if !done && (another || !answer.contains('\n')) {
                    continue;
                }
                let current = answer.clone();
                let request = request.clone();
                let keep_going = this
                    .update(cx, |this, cx| {
                        if this.selection.head != request.offset || this.buffer.version() != request.version {
                            return false;
                        }
                        let completion =
                            if request.use_fill { current.clone() } else { ai::strip_code_fence(&current) };
                        let completion = trim_overlap(&request.before, &request.after, &completion);
                        let mut lines = shape_suggestion(&completion, &request.suffix, request.block);
                        if request.single_line {
                            lines.truncate(1);
                        }
                        if lines.iter().all(|l| l.trim().is_empty()) {
                            // Keep the file's suggestion when the AI has nothing better.
                            return !done;
                        }
                        if another {
                            this.add_alternative(lines, cx);
                        } else {
                            this.set_ghost(lines.clone(), cx);
                            if let Some(ghost) = &mut this.ghost {
                                ghost.request = Some(request.clone());
                            }
                            if done {
                                this.remember_suggestion(request.cache_key.clone(), lines);
                            }
                        }
                        !done
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
            if !request.use_fill {
                ai::log_request(&settings, "chat", started, None, &Ok(()));
            }
        }));
    }

    fn remember_suggestion(&mut self, key: String, lines: Vec<String>) {
        self.ghost_cache.retain(|(k, _)| *k != key);
        self.ghost_cache.push((key, lines));
        if self.ghost_cache.len() > CACHE_SIZE {
            self.ghost_cache.remove(0);
        }
    }

    fn add_alternative(&mut self, lines: Vec<String>, cx: &mut Context<Self>) {
        let Some(ghost) = &mut self.ghost else { return };
        if ghost.alternatives.contains(&lines) {
            return;
        }
        ghost.alternatives.push(lines.clone());
        ghost.current = ghost.alternatives.len() - 1;
        ghost.lines = lines;
        self.rebuild_blocks();
        cx.notify();
    }

    /// Files the current one pulls in (includes, imports, modules), so suggestions know
    /// their names: the start of each, a few at most.
    fn related_files(&self) -> Vec<(String, String)> {
        let Some(path) = &self.path else { return Vec::new() };
        let Some(dir) = path.parent() else { return Vec::new() };
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        // A C file's own header.
        if matches!(path.extension().and_then(|e| e.to_str()), Some("c" | "cpp" | "cc")) {
            candidates.push(path.with_extension("h"));
        }
        let text = self.buffer.slice(0..self.buffer.line_to_char(200));
        for line in text.lines() {
            let line = line.trim();
            let quoted = |l: &str| l.split(['"', '\'']).nth(1).map(str::to_string);
            if let Some(rest) = line.strip_prefix("#include") {
                if let Some(name) = quoted(rest) {
                    for base in [dir.to_path_buf(), dir.join("include"), dir.join("../include")] {
                        candidates.push(base.join(&name));
                    }
                }
            } else if let Some(rest) = line.strip_prefix("from ").or_else(|| line.strip_prefix("import ")) {
                let module = rest.split_whitespace().next().unwrap_or("").trim_start_matches('.');
                if !module.is_empty() {
                    candidates.push(dir.join(format!("{}.py", module.replace('.', "/"))));
                }
            } else if let Some(name) = line.strip_prefix("mod ").and_then(|r| r.strip_suffix(';')) {
                candidates.push(dir.join(format!("{name}.rs")));
                candidates.push(dir.join(name).join("mod.rs"));
            } else if (line.starts_with("import ") || line.contains("require("))
                && let Some(name) = quoted(line).filter(|n| n.starts_with('.'))
            {
                for ext in ["ts", "tsx", "js", "jsx"] {
                    candidates.push(dir.join(format!("{name}.{ext}")));
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        candidates
            .into_iter()
            .filter(|p| p != path && p.is_file())
            .filter_map(|p| p.canonicalize().ok())
            .filter(|p| seen.insert(p.clone()))
            .take(RELATED_FILES)
            .filter_map(|p| {
                let body = std::fs::read_to_string(&p).ok()?;
                let mut end = body.len().min(RELATED_CHARS);
                while !body.is_char_boundary(end) {
                    end -= 1;
                }
                let name = p.strip_prefix(dir.canonicalize().ok()?).unwrap_or(&p).display().to_string();
                Some((name, body[..end].to_string()))
            })
            .collect()
    }

    fn accept_ghost(&mut self, _: &AcceptGhost, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ghost) = self.ghost.take() else { return };
        self.ghost_task = None;
        let text = ghost.lines.join("\n");
        self.edit(ghost.offset..ghost.offset, &text, EditKind::Other, cx);
        self.rebuild_blocks();
    }

    /// Takes the first part of the suggestion; the rest stays suggested.
    fn accept_part(&mut self, take: fn(&str) -> &str, cx: &mut Context<Self>) {
        let Some(ghost) = self.ghost.take() else { return };
        let text = ghost.lines.join("\n");
        let piece = take(&text).to_string();
        let rest = text[piece.len()..].to_string();
        self.ghost_task = None;
        self.edit(ghost.offset..ghost.offset, &piece, EditKind::Typing, cx);
        if !rest.is_empty() {
            let lines = rest.split('\n').map(str::to_string).collect();
            self.ghost = Some(Ghost::new(ghost.offset + piece.chars().count(), self.buffer.version(), lines));
        }
        self.rebuild_blocks();
        cx.notify();
    }

    pub(super) fn accept_ghost_word(&mut self, _: &AcceptGhostWord, _: &mut Window, cx: &mut Context<Self>) {
        self.accept_part(next_word, cx);
    }

    pub(super) fn accept_ghost_line(&mut self, _: &AcceptGhostLine, _: &mut Window, cx: &mut Context<Self>) {
        self.accept_part(next_line, cx);
    }

    /// ⌥⇥: the next option seen here, or a new one from the AI.
    pub(super) fn next_ghost(&mut self, _: &NextGhost, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ghost) = &mut self.ghost else { return };
        if ghost.current + 1 < ghost.alternatives.len() {
            ghost.current += 1;
            ghost.lines = ghost.alternatives[ghost.current].clone();
            self.rebuild_blocks();
            return cx.notify();
        }
        if let Some(request) = ghost.request.clone() {
            self.request_suggestion(request, true, false, cx);
        }
    }

    pub(super) fn previous_ghost(&mut self, _: &PreviousGhost, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ghost) = &mut self.ghost else { return };
        if ghost.current > 0 {
            ghost.current -= 1;
            ghost.lines = ghost.alternatives[ghost.current].clone();
            self.rebuild_blocks();
            cx.notify();
        }
    }

    fn dismiss_ghost(&mut self, _: &DismissGhost, _: &mut Window, cx: &mut Context<Self>) {
        self.ghost = None;
        self.ghost_task = None;
        self.rebuild_blocks();
        cx.notify();
    }

    pub(super) fn accept_ghost_action(&mut self, a: &AcceptGhost, w: &mut Window, cx: &mut Context<Self>) {
        self.accept_ghost(a, w, cx)
    }

    pub(super) fn dismiss_ghost_action(&mut self, a: &DismissGhost, w: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_ghost(a, w, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a check: finding a name to suggest in an 8 MB file, the whole file against
    /// around the caret. `cargo test --release timing -- --ignored --nocapture`
    #[gpui::test]
    #[ignore]
    fn timing_names_in_a_big_file(cx: &mut gpui::TestAppContext) {
        let source = std::fs::read_to_string("src/workspace.rs").unwrap();
        let big = source.repeat((8 << 20) / source.len() + 1);
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(crate::buffer::Buffer::from_text(&big), Some("big.rs".into()), cx));
        e.update(cx, |e, _| {
            e.selection = super::super::Selection::caret(e.buffer.len_chars() / 2);
            let time = |f: &dyn Fn() -> Option<String>| {
                let start = std::time::Instant::now();
                for _ in 0..10 {
                    f();
                }
                start.elapsed() / 10
            };
            let whole = time(&|| best_name(&e.buffer.to_string(), "comp", e.selection.head));
            let near = time(&|| e.best_name_near("comp"));
            println!("names, whole file: {whole:?}; around the caret: {near:?}");
        });
    }

    #[test]
    fn suggests_names_defined_in_the_file() {
        let text = "def compute_moving_average(xs):\n    pass\n\ncomplete = 1\nres = comp";
        assert_eq!(best_name(text, "comp", text.len()).as_deref(), Some("compute_moving_average"));
        assert_eq!(best_name(text, "zz", text.len()), None);
    }

    #[test]
    fn drops_what_the_model_repeated() {
        // It restarted the name being typed.
        assert_eq!(trim_overlap("res = comp", "", "compute_moving_average(xs)"), "ute_moving_average(xs)");
        // It restarted the whole line.
        assert_eq!(trim_overlap("res = comp", "", "res = compute(xs)"), "ute(xs)");
        // A closing bracket that's already there isn't added twice.
        assert_eq!(trim_overlap("print(av", ")", "erage(xs))"), "erage(xs)");
        // ...but a suggestion that closes its own bracket keeps it.
        assert_eq!(trim_overlap("print(av", ")", "erage(xs)"), "erage(xs)");
        // A plain continuation is left alone.
        assert_eq!(trim_overlap("x = ", "", "len(xs)"), "len(xs)");
    }

    #[test]
    fn takes_a_suggestion_a_word_or_a_line_at_a_time() {
        assert_eq!(next_word("_moving_average(xs)"), "_moving_average");
        assert_eq!(next_word("(xs)"), "(");
        assert_eq!(next_word(" + 1"), " +");
        assert_eq!(next_word("\n    write(1)"), "\n    ");
        assert_eq!(next_line("x + 1\nreturn"), "x + 1");
        assert_eq!(next_line("\n    write(1, &c, 1);\n}"), "\n    write(1, &c, 1);");
    }

    #[test]
    fn suggestions_stop_before_repeating_what_follows() {
        let suffix = "\n}\n\nint main(void)";
        let completion = "\n    write(1, &c, 1);\n}\n\nint main(void)";
        // The closing brace is already there: not suggested twice.
        assert_eq!(shape_suggestion(completion, suffix, true), ["", "    write(1, &c, 1);"]);
        // A brace that isn't there yet stays.
        assert_eq!(shape_suggestion("\n    write(1, &c, 1);\n}", "", true), ["", "    write(1, &c, 1);", "}"]);
        assert_eq!(shape_suggestion("x + 1\n\n\n", "", false), ["x + 1"]);
    }
}
