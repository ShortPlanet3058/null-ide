//! Suggestions while typing, shown faintly after the caret; ⇥ takes them.
//!
//! Two sources, fastest first:
//! - names from the file: typing `pro` next to a `process_records` defined above
//!   suggests the rest at once, no AI involved;
//! - the AI, after a short pause, which can go further (the arguments, the rest of
//!   the line) and streams in as it arrives.
//!
//! Typing the characters a suggestion shows keeps it, so it never flickers away.
//! Off unless switched on in Settings → AI.

use super::{EditKind, Editor};
use crate::ai::{self, AiEvent, Prompt, ProviderId};
use crate::settings::Settings;
use futures::StreamExt;
use gpui::{App, Context, KeyBinding, Window, actions};
use std::collections::HashMap;
use std::time::Duration;

actions!(ghost, [AcceptGhost, DismissGhost]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", AcceptGhost, Some("Editor && ai_ghost")),
        KeyBinding::new("escape", DismissGhost, Some("Editor && ai_ghost")),
    ]);
}

/// Wait this long after the last keystroke before asking the AI.
const PAUSE: Duration = Duration::from_millis(250);
const MAX_LINES: usize = 6;
/// The file sent to the AI is cut to about this many characters around the caret.
const CONTEXT_BEFORE: usize = 12_000;
const CONTEXT_AFTER: usize = 3_000;
/// Files the current one includes or imports, given to the AI: at most this many, cut to this size.
const RELATED_FILES: usize = 4;
const RELATED_CHARS: usize = 4_000;
/// Names suggested from the file need at least this much typed.
const MIN_PREFIX: usize = 2;

pub(super) struct Ghost {
    /// Where it goes, and the text it was made for: it disappears once either changes.
    pub(super) offset: usize,
    pub(super) version: u64,
    pub(super) lines: Vec<String>,
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
        self.ghost = (!lines.iter().all(|l| l.is_empty())).then(|| Ghost {
            offset: self.selection.head,
            version: self.buffer.version(),
            lines,
        });
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
            Ghost { offset: ghost.offset + text.chars().count(), version: 0, lines }
        })
    }

    /// After typing: suggest at once from the file, then ask the AI after a pause.
    /// `kept` is a suggestion that what was typed matched, which then stays as it is.
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
        // Only at the end of a line (closing brackets after the caret are fine).
        let (line, col) = self.buffer.point(self.selection.head);
        let line_text = self.buffer.line_text(line);
        let split = line_text.char_indices().nth(col).map_or(line_text.len(), |(b, _)| b);
        let (before, after) = (line_text[..split].to_string(), line_text[split..].to_string());
        if !after.chars().all(|c| c.is_whitespace() || matches!(c, ')' | ']' | '}' | ';' | ',' | '"' | '\'' | ':')) {
            return;
        }

        // 1. A name from the file, right away.
        let word = typed_word(&before);
        if word.chars().count() >= MIN_PREFIX
            && let Some(name) = best_name(&self.buffer.to_string(), word, self.selection.head)
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
        let offset = self.selection.head;
        let version = self.buffer.version();
        let text = self.buffer.to_string();
        let caret_byte = self.buffer.rope().char_to_byte(offset);
        let mut from = caret_byte.saturating_sub(CONTEXT_BEFORE);
        while !text.is_char_boundary(from) {
            from += 1;
        }
        let mut to = (caret_byte + CONTEXT_AFTER).min(text.len());
        while !text.is_char_boundary(to) {
            to -= 1;
        }
        let path = self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "untitled".into());
        let related = self.related_files();
        let suffix_text = text[caret_byte..to].to_string();
        // Code models fill the gap between what's before and after the caret: the best
        // suggestions, and quick. Other models get a chat prompt instead.
        let use_fill = ai::fim_available(&ai_settings) && !fill_failed(&ai_settings);
        let comment = self.language().and_then(|l| l.line_comment).unwrap_or("//");
        let fill = ai::Fim {
            prefix: format!(
                "{}{comment} {path}\n{}",
                related.iter().map(|(name, body)| format!("{comment} {name}\n{body}\n\n")).collect::<String>(),
                &text[from..caret_byte]
            ),
            suffix: suffix_text.clone(),
            max_tokens: 160,
        };
        // A new block (after `:` or `{`, or on an empty line) can take a few lines.
        let block = before.trim().is_empty() || before.trim_end().ends_with([':', '{', '(', '[']);
        let context: String = related
            .iter()
            .map(|(name, body)| format!("Another file of the project, {name}:\n<file>\n{body}\n</file>\n\n"))
            .collect();
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
                "{context}File: {path} ({})\n\n{}<CURSOR>{}\n\nThe current line, up to the cursor: {before:?}",
                self.language_name(),
                &text[from..caret_byte],
                &suffix_text,
            ),
            // A fill-only code model can't chat; the usual model does it then.
            model: ai_settings.completion_model(provider).filter(|_| !ai::fim_available(&ai_settings)),
            max_tokens: Some(160),
        };
        self.ghost_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE).await;
            let still_there = this
                .update(cx, |this, _| this.selection.head == offset && this.buffer.version() == version)
                .unwrap_or(false);
            if !still_there {
                return;
            }
            let started = std::time::Instant::now();
            let mut events = if use_fill {
                ai::stream_fim(ai_settings.clone(), fill)
            } else {
                ai::stream(ai_settings.clone(), chat)
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
                        if use_fill {
                            // That model or endpoint can't fill: use the chat model from now on.
                            mark_fill_failed(&ai_settings);
                        } else {
                            ai::log_request(&ai_settings, "chat", started, None, &Err(message));
                        }
                        return;
                    }
                };
                // Show it as it comes, a line at a time.
                if !done && !answer.contains('\n') {
                    continue;
                }
                let current = answer.clone();
                let suffix = suffix_text.clone();
                let keep_going = this
                    .update(cx, |this, cx| {
                        if this.selection.head != offset || this.buffer.version() != version {
                            return false;
                        }
                        let completion = if use_fill { current.clone() } else { ai::strip_code_fence(&current) };
                        let completion = trim_overlap(&before, &after, &completion);
                        let lines = shape_suggestion(&completion, &suffix, block);
                        // Keep the file's suggestion if the AI has nothing better.
                        if lines.iter().any(|l| !l.trim().is_empty()) {
                            this.set_ghost(lines, cx);
                        }
                        !done
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
            if !use_fill {
                ai::log_request(&ai_settings, "chat", started, None, &Ok(()));
            }
        }));
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
        let text = self.buffer.to_string();
        for line in text.lines().take(200) {
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
