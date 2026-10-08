//! Colours from the language server: what each name in the file is (a type, a function, a
//! macro, a plain variable), where the grammar can only guess from how it's written. They
//! go over the grammar's colours, move with their code while typing, and are asked for
//! again once typing pauses. Swift, with no grammar here, gets most of its colours this way.

use super::Editor;
use crate::highlight::Span;
use crate::lsp_store::Readiness;
use crate::theme::Syntax;
use gpui::{Context, Task};
use std::ops::Range;
use std::time::Duration;

/// Ask again once typing has paused this long.
const PAUSE: Duration = Duration::from_millis(400);
/// While the server starts or reads the project, look again this often.
const NOT_READY: Duration = Duration::from_secs(1);

/// The kinds of names Null colours, as the protocol calls them.
pub const SEMANTIC_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "decorator",
];

#[derive(Default)]
pub(super) struct Meaning {
    /// Bytes in the text and their colour; in order, apart.
    tokens: Vec<Span>,
    /// The buffer revision `tokens` were brought up to.
    revision: u64,
    /// The revision the server last answered for: if it's this one there's nothing to ask.
    answered: Option<u64>,
    /// The servers' refresh count when last asked: when they say names changed (the
    /// project's read), ask again.
    refresh: u64,
    task: Option<Task<()>>,
}

/// Edits moving the colours along at most: past this (thousands of cursors typing), they're
/// dropped and asked for again.
const MAX_EDITS_TO_FOLLOW: usize = 64;
/// An empty answer is asked again this many times, this far apart: a server can say
/// "ready" before it has read the file.
const EMPTY_TRIES: usize = 3;
const EMPTY_AGAIN: Duration = Duration::from_secs(2);

/// The colour for a kind of name, by its name in the server's legend (servers add their
/// own: rust-analyzer's `builtinType`, `selfKeyword`…). None: the grammar's colour stays.
fn syntax_of(kind: &str) -> Option<Syntax> {
    Some(match kind {
        "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" | "typeAlias" | "builtinType"
        | "union" | "trait" | "selfTypeKeyword" | "enumMember" | "concept" => Syntax::Type,
        "function" | "method" => Syntax::Function,
        "macro" | "derive" | "deriveHelper" => Syntax::Macro,
        "decorator" | "attribute" | "builtinAttribute" => Syntax::Attribute,
        "property" => Syntax::Property,
        "keyword" | "modifier" | "selfKeyword" => Syntax::Keyword,
        "variable" | "parameter" => Syntax::Plain,
        _ => return None,
    })
}

/// The server's tokens as (line, UTF-16 column, UTF-16 length, colour), the ones Null
/// colours only. Code in a comment (a doc example) keeps the comment's colour.
fn decode(tokens: &[lsp_types::SemanticToken], legend: &crate::lsp::SemanticLegend) -> Vec<(u32, u32, u32, Syntax)> {
    let injected = legend.modifiers.iter().position(|m| m == "injected");
    let (mut line, mut start) = (0u32, 0u32);
    tokens
        .iter()
        .filter_map(|t| {
            if t.delta_line > 0 {
                line += t.delta_line;
                start = t.delta_start;
            } else {
                start += t.delta_start;
            }
            if injected.is_some_and(|bit| bit < 32 && t.token_modifiers_bitset & (1 << bit) != 0) {
                return None;
            }
            let syntax = syntax_of(legend.types.get(t.token_type as usize)?)?;
            (t.length > 0).then_some((line, start, t.length, syntax))
        })
        .collect()
}

/// Decoded tokens as bytes in `buffer`'s text, in order and apart. Each line is read once,
/// its tokens found along it in order (a minified file's one line can hold 100k of them).
fn spans_in(tokens: &[(u32, u32, u32, Syntax)], buffer: &crate::buffer::Buffer) -> Vec<Span> {
    let rope = buffer.rope();
    let lines = rope.len_lines();
    let mut spans: Vec<Span> = Vec::with_capacity(tokens.len());
    let mut current: Option<(u32, usize, String)> = None;
    // Where the walk along the line is: its byte, and its UTF-16 unit.
    let (mut byte, mut unit) = (0usize, 0usize);
    for &(line, start, len, syntax) in tokens {
        if line as usize >= lines {
            break;
        }
        if current.as_ref().is_none_or(|(l, ..)| *l != line) {
            let text = rope.line(line as usize).to_string();
            current = Some((line, rope.line_to_byte(line as usize), text));
            (byte, unit) = (0, 0);
        }
        let Some((_, line_byte, text)) = &current else { break };
        // Tokens come in order along a line; one that doesn't is dropped.
        let (start, end) = (start as usize, start as usize + len as usize);
        if start < unit {
            continue;
        }
        let walk = |to: usize, byte: &mut usize, unit: &mut usize| {
            for c in text[*byte..].chars() {
                if *unit >= to || c == '\n' || c == '\r' {
                    break;
                }
                *unit += c.len_utf16();
                *byte += c.len_utf8();
            }
        };
        walk(start, &mut byte, &mut unit);
        let from = byte;
        walk(end, &mut byte, &mut unit);
        if from < byte {
            spans.push((line_byte + from..line_byte + byte, syntax));
        }
    }
    // A name followed by one `:` is a label (Swift's `f(p: x)`), never a call.
    spans.retain(|(r, syntax)| {
        !(*syntax == Syntax::Function && rope.get_byte(r.end) == Some(b':') && rope.get_byte(r.end + 1) != Some(b':'))
    });
    spans
}

/// The grammar's `spans` with `tokens` laid over them, both in order and each apart.
pub(super) fn overlay(spans: &[Span], tokens: &[Span]) -> Vec<Span> {
    let mut out = Vec::with_capacity(spans.len() + tokens.len());
    for (range, syntax) in spans {
        let mut start = range.start;
        let from = tokens.partition_point(|(t, _)| t.end <= range.start);
        for (t, _) in tokens[from..].iter().take_while(|(t, _)| t.start < range.end) {
            if t.start > start {
                out.push((start..t.start, *syntax));
            }
            start = start.max(t.end);
        }
        if start < range.end {
            out.push((start..range.end, *syntax));
        }
    }
    out.extend(tokens.iter().cloned());
    out.sort_by_key(|(r, _)| r.start);
    out
}

/// Where a token is after `edit`: along with its code; gone if the edit touched it.
fn map_token(range: Range<usize>, edit: &crate::buffer::Edit) -> Option<Range<usize>> {
    let (start, old_end, new_end) = (edit.start_byte, edit.old_end_byte, edit.new_end_byte);
    if range.end <= start {
        Some(range)
    } else if range.start >= old_end && !(range.start == start && start == old_end) {
        Some(range.start - old_end + new_end..range.end - old_end + new_end)
    } else {
        None
    }
}

impl Editor {
    /// The colours to draw `shown` (bytes) with, when the server's change the grammar's:
    /// those, with the server's over them. None: the grammar's alone.
    pub fn spans_with_meaning(&self, shown: Range<usize>) -> Option<Vec<Span>> {
        if self.meaning.tokens.is_empty() || self.meaning.revision != self.buffer.revision() {
            return None;
        }
        let tokens = &self.meaning.tokens;
        let from = tokens.partition_point(|(r, _)| r.end <= shown.start);
        let to = from + tokens[from..].partition_point(|(r, _)| r.start < shown.end);
        // Only what's drawn: this runs every frame.
        let first = self.spans.partition_point(|(r, _)| r.end <= shown.start);
        let last = first + self.spans[first..].partition_point(|(r, _)| r.start < shown.end);
        (from < to).then(|| overlay(&self.spans[first..last], &tokens[from..to]))
    }

    /// A line's colours (`line_spans`), with the server's over them.
    pub fn line_spans_with_meaning(&self, line: usize, spans: Vec<Span>) -> Vec<Span> {
        if self.meaning.tokens.is_empty() || self.meaning.revision != self.buffer.revision() {
            return spans;
        }
        let shown = self.buffer.line_to_byte(line)..self.buffer.line_to_byte(line + 1);
        let tokens = &self.meaning.tokens;
        let from = tokens.partition_point(|(r, _)| r.end <= shown.start);
        let to = from + tokens[from..].partition_point(|(r, _)| r.start < shown.end);
        if from == to { spans } else { overlay(&spans, &tokens[from..to]) }
    }

    /// After an edit: colours move with their code, and the server is asked again later.
    pub(super) fn meaning_after_edit(&mut self) {
        let revision = self.buffer.revision();
        if self.meaning.revision == revision {
            return;
        }
        match self.buffer.edits_since(self.meaning.revision) {
            Some(edits) => {
                let edits: Vec<_> = edits.take(MAX_EDITS_TO_FOLLOW + 1).collect();
                if edits.len() > MAX_EDITS_TO_FOLLOW {
                    self.meaning.tokens.clear();
                }
                self.meaning.tokens.retain_mut(|(range, _)| {
                    match edits.iter().try_fold(range.clone(), |r, e| map_token(r, e)) {
                        Some(r) => {
                            *range = r;
                            true
                        }
                        None => false,
                    }
                });
            }
            None => self.meaning.tokens.clear(),
        }
        self.meaning.revision = revision;
        self.meaning.task = None;
    }

    /// Asks the server what the names in this text are, once typing pauses.
    pub fn ensure_meaning(&mut self, cx: &mut Context<Self>) {
        let revision = self.buffer.revision();
        let Some(lsp) = self.lsp.clone() else { return };
        let refresh = lsp.read(cx).semantic_refresh();
        if self.meaning.task.is_some() || (self.meaning.answered == Some(revision) && self.meaning.refresh == refresh) {
            return;
        }
        let Some(path) = self.path.clone() else { return };
        if !lsp.read(cx).has_server_for(&path) {
            return;
        }
        self.meaning.refresh = refresh;
        self.meaning.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE).await;
            loop {
                let Ok(readiness) = this.update(cx, |this, cx| this.readiness(cx)) else { return };
                match readiness {
                    Some(Readiness::Ready { .. }) => break,
                    Some(Readiness::Starting | Readiness::Indexing { .. } | Readiness::Installing { .. }) => {
                        cx.background_executor().timer(NOT_READY).await;
                    }
                    _ => {
                        this.update(cx, |this, _| {
                            this.meaning.answered = Some(revision);
                            this.meaning.task = None;
                        })
                        .ok();
                        return;
                    }
                }
            }
            for attempt in 1..=EMPTY_TRIES {
                let Ok(request) = this.update(cx, |_, cx| lsp.read(cx).semantic_tokens(&path)) else { return };
                let found = request.await;
                let Ok(empty) = this.update(cx, |this, cx| {
                    if this.buffer.revision() != revision {
                        this.meaning.task = None;
                        return false;
                    }
                    let tokens = found.map(|(data, legend)| decode(&data, &legend)).unwrap_or_default();
                    if tokens.is_empty() && attempt < EMPTY_TRIES {
                        return true;
                    }
                    this.meaning.tokens = spans_in(&tokens, &this.buffer);
                    this.meaning.revision = revision;
                    this.meaning.answered = Some(revision);
                    this.meaning.task = None;
                    cx.notify();
                    false
                }) else {
                    return;
                };
                if !empty {
                    return;
                }
                cx.background_executor().timer(EMPTY_AGAIN).await;
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::SemanticLegend;
    use lsp_types::SemanticToken;

    fn token(delta_line: u32, delta_start: u32, length: u32, token_type: u32, modifiers: u32) -> SemanticToken {
        SemanticToken { delta_line, delta_start, length, token_type, token_modifiers_bitset: modifiers }
    }

    #[test]
    fn tokens_decode_by_the_server_s_legend() {
        let legend = SemanticLegend {
            types: ["struct", "function", "variable", "string", "builtinType"].map(String::from).to_vec(),
            modifiers: ["declaration", "injected"].map(String::from).to_vec(),
        };
        let tokens = [
            token(0, 7, 3, 0, 0),
            token(0, 4, 5, 1, 1),
            token(0, 6, 4, 3, 0),
            token(2, 2, 3, 4, 0),
            token(0, 4, 1, 2, 0),
            token(1, 0, 3, 1, 2),
        ];
        assert_eq!(
            decode(&tokens, &legend),
            [(0, 7, 3, Syntax::Type), (0, 11, 5, Syntax::Function), (2, 2, 3, Syntax::Type), (2, 6, 1, Syntax::Plain),],
            "strings keep the grammar's colour; code in a comment, the comment's"
        );
    }

    #[test]
    fn tokens_are_found_along_their_lines() {
        let buffer = crate::buffer::Buffer::from_text("let é = f(p: 1);\r\nx\n");
        let tokens = [
            (0, 4, 1, Syntax::Plain),
            (0, 8, 1, Syntax::Function),
            (0, 6, 1, Syntax::Type),
            (0, 10, 1, Syntax::Function),
            (0, 30, 2, Syntax::Type),
            (1, 0, 1, Syntax::Type),
            (9, 0, 1, Syntax::Type),
        ];
        assert_eq!(
            spans_in(&tokens, &buffer),
            [(4..6, Syntax::Plain), (9..10, Syntax::Function), (19..20, Syntax::Type)],
            "é is two bytes; out of order, past the line, a label, past the text: dropped"
        );
    }

    #[test]
    fn tokens_go_over_the_grammar_s_colours() {
        let spans = [(0..3, Syntax::Keyword), (4..12, Syntax::Type), (20..25, Syntax::String)];
        let tokens = [(6..9, Syntax::Function), (14..16, Syntax::Macro)];
        assert_eq!(
            overlay(&spans, &tokens),
            [
                (0..3, Syntax::Keyword),
                (4..6, Syntax::Type),
                (6..9, Syntax::Function),
                (9..12, Syntax::Type),
                (14..16, Syntax::Macro),
                (20..25, Syntax::String),
            ]
        );
    }

    #[test]
    fn tokens_move_with_their_code() {
        let edit = |start: usize, old_end: usize, new_end: usize| crate::buffer::Edit {
            start_byte: start,
            old_end_byte: old_end,
            new_end_byte: new_end,
            start: (0, 0),
            old_end: (0, 0),
            new_end: (0, 0),
            lsp_range: None,
            text: String::new(),
        };
        assert_eq!(map_token(4..8, &edit(0, 0, 2)), Some(6..10));
        assert_eq!(map_token(4..8, &edit(9, 9, 10)), Some(4..8));
        assert_eq!(map_token(4..8, &edit(8, 8, 9)), Some(4..8), "typing right after it: it doesn't grow");
        assert_eq!(map_token(4..8, &edit(4, 4, 5)), None, "typing at its start: asked again");
        assert_eq!(map_token(4..8, &edit(5, 6, 5)), None);
    }
}

/// Against the real servers: `cargo test real_servers_colour_names -- --ignored`.
#[cfg(test)]
mod real {
    use super::*;
    use crate::lsp::{LanguageServer, ServerMessage, uri_for};
    use futures::StreamExt;
    use lsp_types::notification::{DidOpenTextDocument, Initialized};
    use lsp_types::request::Initialize;
    use lsp_types::*;
    use std::path::Path;
    use std::time::{Duration, Instant};

    /// The colour the server gives each of `words` (its first time in `source`).
    fn colours(program: &str, dir: &Path, file: &Path, language: &str, source: &str, words: &[&str]) -> Vec<Syntax> {
        let (server, mut messages) = LanguageServer::spawn(Path::new(program), &[], dir).unwrap();
        futures::executor::block_on(async {
            #[allow(deprecated)]
            let init = InitializeParams { root_uri: uri_for(dir), ..Default::default() };
            let result = server.request::<Initialize>(init).await.unwrap();
            server.set_capabilities(&result.capabilities);
            server.notify::<Initialized>(InitializedParams {});
            server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri_for(file).unwrap(),
                    language_id: language.into(),
                    version: 0,
                    text: source.into(),
                },
            });
            let legend = server.semantic_legend().expect("semantic tokens");
            let deadline = Instant::now() + Duration::from_secs(180);
            loop {
                // Answer what the server asks meanwhile.
                while let Some(Some(message)) = messages.next().now_or_never() {
                    if let ServerMessage::Request { id, .. } = message {
                        server.respond(id, serde_json::Value::Null);
                    }
                }
                let reply = server
                    .request::<request::SemanticTokensFullRequest>(SemanticTokensParams {
                        text_document: TextDocumentIdentifier { uri: uri_for(file).unwrap() },
                        work_done_progress_params: Default::default(),
                        partial_result_params: Default::default(),
                    })
                    .await;
                let data = match reply {
                    Ok(Some(SemanticTokensResult::Tokens(t))) => t.data,
                    _ => Vec::new(),
                };
                let spans = spans_in(&decode(&data, &legend), &crate::buffer::Buffer::from_text(source));
                let found: Vec<Option<Syntax>> = words
                    .iter()
                    .map(|word| {
                        let at = source.find(word)?;
                        let name = word.split(|c: char| !c.is_alphanumeric()).next().unwrap_or(word);
                        Some(spans.iter().find(|(r, _)| *r == (at..at + name.len())).map_or(Syntax::Note, |t| t.1))
                    })
                    .collect();
                // Until the server has read the file (an answer with something for the first word).
                if found.first().is_some_and(|f| *f != Some(Syntax::Note)) || Instant::now() > deadline {
                    return found.into_iter().map(|s| s.unwrap_or(Syntax::Note)).collect();
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        })
    }

    use futures::FutureExt;

    #[test]
    #[ignore]
    fn real_servers_colour_names() {
        let dir = crate::tools::test_dir("meaning-ra");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
            .unwrap();
        let source =
            "struct Point { x: i32 }\n\nfn main() {\n    let p = Point { x: 1 };\n    println!(\"{}\", p.x);\n}\n";
        let file = dir.join("src/main.rs");
        std::fs::write(&file, source).unwrap();
        let program = std::env::var("HOME").unwrap() + "/.cargo/bin/rust-analyzer";
        let got = colours(&program, &dir, &file, "rust", source, &["Point", "main", "println", "p ="]);
        assert_eq!(got, [Syntax::Type, Syntax::Function, Syntax::Macro, Syntax::Plain]);
        std::fs::remove_dir_all(&dir).ok();

        let dir = crate::tools::test_dir("meaning-swift");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = "struct Point {\n    var x: Int\n}\n\nfunc show(p: Point) {\n    print(p.x)\n}\n";
        let file = dir.join("main.swift");
        std::fs::write(&file, source).unwrap();
        let got =
            colours("/usr/bin/sourcekit-lsp", &dir, &file, "swift", source, &["Int", "print", "Point)", "p: Point"]);
        // A label isn't a call: no colour from the server, the grammar's stays.
        assert_eq!(got, [Syntax::Type, Syntax::Function, Syntax::Type, Syntax::Note]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
