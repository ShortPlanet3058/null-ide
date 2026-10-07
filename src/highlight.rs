use crate::buffer::{Buffer, Edit};
use crate::languages::Language;
use crate::theme::Syntax;
use ropey::Rope;
use std::ops::Range;
use tree_sitter::{InputEdit, Parser, Point, Query, QueryCursor, StreamingIterator, TextProvider, Tree};

/// Capture names we ask tree-sitter for. A capture like `comment.documentation`
/// falls back to the longest matching prefix, here `comment`.
pub const CAPTURES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "delimiter",
    "embedded",
    "escape",
    "function",
    "function.builtin",
    "function.macro",
    "function.method",
    "function.special",
    "keyword",
    "label",
    "module",
    "namespace",
    "number",
    "operator",
    "property",
    "punctuation",
    "string",
    "string.special",
    "string.special.key",
    "tag",
    "text.literal",
    "text.reference",
    "text.title",
    "text.uri",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
    // CSS at-rules.
    "charset",
    "import",
    "keyframes",
    "media",
    "supports",
];

fn syntax_for(capture: &str) -> Syntax {
    match capture {
        "attribute" => Syntax::Attribute,
        "comment" => Syntax::Comment,
        "boolean" | "constant" | "constant.builtin" | "number" => Syntax::Number,
        "constructor" | "type" | "type.builtin" | "tag" | "module" | "namespace" => Syntax::Type,
        "escape" | "string" | "string.special" | "text.literal" | "text.uri" => Syntax::String,
        "function" | "function.builtin" | "function.method" | "function.special" | "text.reference" => Syntax::Function,
        "function.macro" => Syntax::Macro,
        "keyword" | "label" | "variable.builtin" | "text.title" | "charset" | "import" | "keyframes" | "media"
        | "supports" => Syntax::Keyword,
        "operator" | "punctuation" | "delimiter" => Syntax::Punctuation,
        "property" | "string.special.key" => Syntax::Property,
        _ => Syntax::Plain,
    }
}

/// Byte range in the document and how to color it. Spans are sorted and
/// never overlap.
pub type Span = (Range<usize>, Syntax);

/// A language's highlighting rules, compiled once, with what each capture colours.
pub struct HighlightQuery {
    query: Query,
    /// For each capture: its colour, or None for names Null doesn't colour.
    syntax: Vec<Option<Syntax>>,
}

impl HighlightQuery {
    pub fn new(grammar: &tree_sitter::Language, source: &str) -> Result<Self, tree_sitter::QueryError> {
        let query = Query::new(grammar, source)?;
        let syntax = query.capture_names().iter().map(|name| resolve(name)).collect();
        Ok(Self { query, syntax })
    }
}

/// A capture like `comment.documentation` falls back to the longest known prefix, here `comment`.
fn resolve(name: &str) -> Option<Syntax> {
    let mut name = name;
    loop {
        if CAPTURES.contains(&name) {
            return Some(syntax_for(name));
        }
        name = &name[..name.rfind('.')?];
    }
}

/// One file's syntax tree, kept up to date edit by edit: after a keystroke only the
/// changed part is parsed again, and only the lines on screen are coloured.
pub struct Highlighter {
    query: &'static HighlightQuery,
    parser: Parser,
    tree: Option<Tree>,
    /// The buffer revision the tree matches.
    revision: u64,
}

impl Highlighter {
    pub fn new(language: &'static Language) -> Option<Self> {
        let query = language.highlight_query()?;
        let mut parser = Parser::new();
        parser.set_language(&language.grammar()).ok()?;
        Some(Self { query, parser, tree: None, revision: 0 })
    }

    /// Brings the tree up to date with the text: the edits since last time are applied
    /// to the old tree, so parsing again reuses everything they didn't touch.
    pub fn sync(&mut self, buffer: &Buffer) {
        if self.tree.is_some() && self.revision == buffer.revision() {
            return;
        }
        let mut old = self.tree.take();
        if let Some(tree) = &mut old {
            match buffer.edits_since(self.revision) {
                Some(edits) => edits.for_each(|e| tree.edit(&input_edit(e))),
                None => old = None,
            }
        }
        let rope = buffer.rope();
        let mut read = |byte: usize, _: Point| -> &[u8] {
            if byte >= rope.len_bytes() {
                return &[];
            }
            let (chunk, start, _, _) = rope.chunk_at_byte(byte);
            &chunk.as_bytes()[byte - start..]
        };
        self.tree = self.parser.parse_with_options(&mut read, old.as_ref(), None);
        self.revision = buffer.revision();
    }

    /// The syntax tree, as of the last [`Self::sync`].
    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    /// The coloured spans within `range` (bytes). Call [`Self::sync`] first.
    pub fn spans(&self, rope: &Rope, range: Range<usize>) -> Vec<Span> {
        let Some(tree) = &self.tree else { return Vec::new() };
        let range = range.start.min(rope.len_bytes())..range.end.min(rope.len_bytes());
        if range.is_empty() {
            return Vec::new();
        }
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(range.clone());
        // Every captured node once. A node's captures come in pattern order and the last
        // one decides, as in tree-sitter's own highlighter: queries list general rules
        // (`(identifier) @variable`) before specific ones (`name: (identifier) @function`).
        let mut nodes: Vec<(usize, Range<usize>, Option<Syntax>)> = Vec::new();
        let mut captures = cursor.captures(&self.query.query, tree.root_node(), RopeText(rope));
        while let Some((m, i)) = captures.next() {
            let capture = m.captures[*i];
            let syntax = self.query.syntax[capture.index as usize];
            match nodes.last_mut() {
                Some((id, _, last)) if *id == capture.node.id() => *last = syntax,
                _ => nodes.push((capture.node.id(), capture.node.byte_range(), syntax)),
            }
        }
        let mut nodes: Vec<(Range<usize>, Syntax)> =
            nodes.into_iter().filter_map(|(_, r, syntax)| Some((r, syntax?))).collect();
        // Outer nodes first, so the nodes inside them paint over them.
        nodes.sort_by_key(|(r, _)| (r.start, std::cmp::Reverse(r.end)));
        let mut painted: Vec<Option<Syntax>> = vec![None; range.len()];
        for (r, syntax) in nodes {
            let (a, b) = (r.start.max(range.start) - range.start, r.end.min(range.end).max(range.start) - range.start);
            if a < b {
                painted[a..b].fill(Some(syntax));
            }
        }
        let mut spans: Vec<Span> = Vec::new();
        for (i, syntax) in painted.into_iter().enumerate() {
            let Some(syntax) = syntax.filter(|s| *s != Syntax::Plain) else { continue };
            let at = range.start + i;
            match spans.last_mut() {
                Some((r, s)) if r.end == at && *s == syntax => r.end = at + 1,
                _ => spans.push((at..at + 1, syntax)),
            }
        }
        notes_in_comments(rope, spans)
    }
}

/// The words that leave a note in a comment.
const NOTES: &[&str] = &["TODO", "FIXME", "HACK", "XXX"];

/// Comment spans split around the notes in them (TODO, FIXME…), which stand out.
fn notes_in_comments(rope: &Rope, spans: Vec<Span>) -> Vec<Span> {
    let mut out = Vec::with_capacity(spans.len());
    for (range, syntax) in spans {
        if syntax != Syntax::Comment {
            out.push((range, syntax));
            continue;
        }
        let text = rope.byte_slice(range.clone()).to_string();
        let mut at = 0;
        for (start, word) in note_words(&text) {
            if start > at {
                out.push((range.start + at..range.start + start, Syntax::Comment));
            }
            out.push((range.start + start..range.start + start + word.len(), Syntax::Note));
            at = start + word.len();
        }
        if at < text.len() {
            out.push((range.start + at..range.end, Syntax::Comment));
        }
    }
    out
}

/// Where the note words are in `text`, whole words only (not TODOS, not XXXL).
fn note_words(text: &str) -> Vec<(usize, &'static str)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    for word in NOTES {
        for (start, _) in text.match_indices(word) {
            let end = start + word.len();
            let alone = |b: Option<&u8>| b.is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_');
            if alone(start.checked_sub(1).and_then(|i| bytes.get(i))) && alone(bytes.get(end)) {
                found.push((start, *word));
            }
        }
    }
    found.sort_by_key(|(start, _)| *start);
    found
}

impl Highlighter {
    /// Code that can fold, as line ranges: from the line a block opens on to the line it
    /// closes on, at least one line apart (the lines between are what folding hides). One
    /// per opening line, the longest. Call [`Self::sync`] first.
    /// The foldable blocks (as [`Self::fold_ranges`] finds them) that start above `line`
    /// and end below it, innermost first: a walk up from the line, not over the file.
    pub fn blocks_around(&self, line: usize) -> Vec<Range<usize>> {
        let Some(tree) = &self.tree else { return Vec::new() };
        let point = Point { row: line, column: 0 };
        let Some(mut node) = tree.root_node().descendant_for_point_range(point, point) else { return Vec::new() };
        let mut found = Vec::new();
        loop {
            let (start, end) = (node.start_position().row, node.end_position().row);
            let Some(parent) = node.parent() else { break };
            if node.is_named() && end > start + 1 && start < line && line < end {
                found.push(start..end);
            }
            node = parent;
        }
        found
    }

    pub fn fold_ranges(&self) -> Vec<Range<usize>> {
        let Some(tree) = &self.tree else { return Vec::new() };
        let mut ends: std::collections::BTreeMap<usize, usize> = Default::default();
        let mut cursor = tree.walk();
        'walk: loop {
            let node = cursor.node();
            let (start, end) = (node.start_position().row, node.end_position().row);
            // A node on one or two lines has nothing to hide; neither does the whole file.
            if node.is_named() && end > start + 1 && node.parent().is_some() {
                let longest = ends.entry(start).or_insert(end);
                *longest = (*longest).max(end);
            }
            if end > start + 1 && cursor.goto_first_child() {
                continue;
            }
            while !cursor.goto_next_sibling() {
                if !cursor.goto_parent() {
                    break 'walk;
                }
            }
        }
        ends.into_iter().map(|(start, end)| start..end).collect()
    }
}

fn input_edit(e: &Edit) -> InputEdit {
    let point = |(row, column): (usize, usize)| Point { row, column };
    InputEdit {
        start_byte: e.start_byte,
        old_end_byte: e.old_end_byte,
        new_end_byte: e.new_end_byte,
        start_position: point(e.start),
        old_end_position: point(e.old_end),
        new_end_position: point(e.new_end),
    }
}

/// Node text for query predicates (`#match?`, `#eq?`), read straight from the rope.
struct RopeText<'a>(&'a Rope);

impl<'a> TextProvider<&'a [u8]> for RopeText<'a> {
    type I = std::iter::Map<ropey::iter::Chunks<'a>, fn(&'a str) -> &'a [u8]>;

    fn text(&mut self, node: tree_sitter::Node) -> Self::I {
        let len = self.0.len_bytes();
        let range = node.start_byte().min(len)..node.end_byte().min(len);
        self.0.byte_slice(range).chunks().map(str::as_bytes as fn(&'a str) -> &'a [u8])
    }
}

/// The spans that overlap `range`, clipped to it and shifted so they start
/// at zero.
pub fn spans_in(spans: &[Span], range: Range<usize>) -> impl Iterator<Item = Span> + '_ {
    let first = spans.partition_point(|(r, _)| r.end <= range.start);
    spans[first..]
        .iter()
        .take_while(move |(r, _)| r.start < range.end)
        .map(move |(r, s)| (r.start.max(range.start) - range.start..r.end.min(range.end) - range.start, *s))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The whole text's spans, as the editor would get them.
    pub fn highlight(file: &str, source: &str) -> Vec<Span> {
        let language = crate::languages::for_path(std::path::Path::new(file)).unwrap();
        let mut highlighter = Highlighter::new(language).unwrap();
        let buffer = Buffer::from_text(source);
        highlighter.sync(&buffer);
        highlighter.spans(buffer.rope(), 0..source.len())
    }

    #[test]
    fn edits_reparse_only_what_changed_and_match_a_fresh_parse() {
        let language = crate::languages::for_path(std::path::Path::new("a.rs")).unwrap();
        let mut highlighter = Highlighter::new(language).unwrap();
        let mut buffer = Buffer::from_text("fn main() {\n    let x = 1;\n}\n");
        highlighter.sync(&buffer);
        // Typing a quote turns the rest of the line into a string, then closing it ends it.
        let at = buffer.offset(1, 12);
        buffer.replace(at..at, "\"é");
        let end = buffer.offset(1, 14);
        buffer.replace(end..end, "\"");
        highlighter.sync(&buffer);
        let text = buffer.to_string();
        let incremental = highlighter.spans(buffer.rope(), 0..text.len());
        assert_eq!(incremental, highlight("a.rs", &text));
        assert!(
            incremental.iter().any(|(r, s)| &text[r.clone()] == "\"é\"" && *s == Syntax::String),
            "{incremental:?}"
        );
    }

    #[test]
    fn blocks_fold_from_their_first_line_to_their_last() {
        let language = crate::languages::for_path(std::path::Path::new("a.rs")).unwrap();
        let mut highlighter = Highlighter::new(language).unwrap();
        let buffer = Buffer::from_text("fn a() {\n    if x {\n        y();\n    }\n}\nfn b() {}\n");
        highlighter.sync(&buffer);
        assert_eq!(highlighter.fold_ranges(), vec![0..4, 1..3]);
    }

    #[test]
    fn only_the_asked_range_is_coloured() {
        let source = "fn a() {}\nfn b() {}\n";
        let language = crate::languages::for_path(std::path::Path::new("a.rs")).unwrap();
        let mut highlighter = Highlighter::new(language).unwrap();
        let buffer = Buffer::from_text(source);
        highlighter.sync(&buffer);
        let spans = highlighter.spans(buffer.rope(), 10..source.len());
        assert!(spans.iter().all(|(r, _)| r.start >= 10), "{spans:?}");
        assert!(spans.iter().any(|(r, s)| &source[r.clone()] == "b" && *s == Syntax::Function));
    }

    #[test]
    fn highlights_keywords_strings_and_comments() {
        let source = "// hi\nfn main() { let s = \"x\"; }";
        let spans = highlight("a.rs", source);
        let kind_of = |text: &str| {
            let start = source.find(text).unwrap();
            spans.iter().find(|(r, _)| r.start == start).map(|(_, s)| *s)
        };
        assert_eq!(kind_of("// hi"), Some(Syntax::Comment));
        assert_eq!(kind_of("fn"), Some(Syntax::Keyword));
        assert_eq!(kind_of("main"), Some(Syntax::Function));
        assert_eq!(kind_of("\"x\""), Some(Syntax::String));
    }

    #[test]
    fn spans_are_clipped_to_a_line() {
        let spans = vec![(0..4, Syntax::Keyword), (6..12, Syntax::String)];
        let line: Vec<_> = spans_in(&spans, 8..10).collect();
        assert_eq!(line, vec![(0..2, Syntax::String)]);
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;
    use std::path::Path;

    fn kinds(file: &str, source: &str) -> Vec<(String, Syntax)> {
        let language = crate::languages::for_path(Path::new(file)).unwrap();
        let _ = language;
        super::tests::highlight(file, source).into_iter().map(|(r, s)| (source[r].to_string(), s)).collect()
    }

    #[test]
    fn highlights_python() {
        let spans = kinds("a.py", "def greet(name):\n    # hi\n    return f\"hi {name}\"\n");
        assert!(spans.contains(&("def".into(), Syntax::Keyword)), "{spans:?}");
        assert!(spans.contains(&("greet".into(), Syntax::Function)), "{spans:?}");
        assert!(spans.contains(&("# hi".into(), Syntax::Comment)), "{spans:?}");
    }

    #[test]
    fn highlights_typescript_with_javascript_rules() {
        let spans = kinds("a.ts", "const n: number = 1;\ninterface A {}\n");
        assert!(spans.contains(&("const".into(), Syntax::Keyword)), "{spans:?}");
        assert!(spans.contains(&("1".into(), Syntax::Number)), "{spans:?}");
        assert!(spans.iter().any(|(t, s)| t == "interface" && *s == Syntax::Keyword), "{spans:?}");
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    /// Not a check: how long the work after each keystroke takes on a big file.
    /// `cargo test --release timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn keystroke_costs_on_a_big_file() {
        let source = std::fs::read_to_string("src/workspace.rs").unwrap();
        let language = crate::languages::for_path(std::path::Path::new("a.rs")).unwrap();
        let mut highlighter = Highlighter::new(language).unwrap();
        let mut buffer = Buffer::from_text(&source);
        highlighter.sync(&buffer);
        let at = buffer.offset(3000, 8);
        let mut sync = std::time::Duration::ZERO;
        let mut folds = std::time::Duration::ZERO;
        let mut blocks = std::time::Duration::ZERO;
        for i in 0..20 {
            buffer.replace(at + i..at + i, "x");
            let t = std::time::Instant::now();
            highlighter.sync(&buffer);
            sync += t.elapsed();
            let t = std::time::Instant::now();
            let ranges = highlighter.fold_ranges();
            folds += t.elapsed();
            assert!(!ranges.is_empty());
            let t = std::time::Instant::now();
            let around = highlighter.blocks_around(3000);
            blocks += t.elapsed();
            // The same blocks as the whole-file walk finds around that line.
            let mut expected: Vec<_> = ranges.into_iter().filter(|r| r.start < 3000 && 3000 < r.end).collect();
            let mut got = around.clone();
            got.sort_by_key(|r| r.start);
            got.dedup_by_key(|r| r.start);
            expected.sort_by_key(|r| r.start);
            assert_eq!(
                got.iter().map(|r| r.start).collect::<Vec<_>>(),
                expected.iter().map(|r| r.start).collect::<Vec<_>>()
            );
        }
        // Colours for the lines around the view (as many as the editor keeps: 120 each side).
        let around = buffer.line_to_byte(3000 - 160)..buffer.line_to_byte(3000 + 160);
        let t = std::time::Instant::now();
        for _ in 0..20 {
            std::hint::black_box(highlighter.spans(buffer.rope(), around.clone()));
        }
        println!("colours for 320 lines: {:?}", t.elapsed() / 20);
        println!(
            "per keystroke: reparse {:?}, all fold ranges {:?}, blocks around a line {:?}",
            sync / 20,
            folds / 20,
            blocks / 20
        );
    }
}

#[cfg(test)]
mod note_tests {
    use super::*;

    #[test]
    fn notes_are_whole_words() {
        let text = "// TODO: fix, then FIXME(me) and TODOS, XXXL, HACK";
        let words: Vec<&str> = note_words(text).into_iter().map(|(_, w)| w).collect();
        assert_eq!(words, ["TODO", "FIXME", "HACK"]);
    }
}
