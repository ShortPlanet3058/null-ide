use crate::theme::Syntax;
use std::ops::Range;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter as TsHighlighter};

/// Capture names we ask tree-sitter for. A capture like `comment.documentation`
/// falls back to the longest matching prefix, here `comment`.
const CAPTURES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "escape",
    "function",
    "function.macro",
    "function.method",
    "keyword",
    "label",
    "operator",
    "property",
    "punctuation",
    "string",
    "type",
    "variable.builtin",
];

fn syntax_for(capture: &str) -> Syntax {
    match capture {
        "attribute" => Syntax::Attribute,
        "comment" => Syntax::Comment,
        "constant" | "constant.builtin" => Syntax::Number,
        "constructor" | "type" => Syntax::Type,
        "escape" | "string" => Syntax::String,
        "function" | "function.method" => Syntax::Function,
        "function.macro" => Syntax::Macro,
        "keyword" | "label" | "variable.builtin" => Syntax::Keyword,
        "operator" | "punctuation" => Syntax::Punctuation,
        "property" => Syntax::Property,
        _ => Syntax::Plain,
    }
}

/// Byte range in the document and how to color it. Spans are sorted and
/// never overlap.
pub type Span = (Range<usize>, Syntax);

pub struct Highlighter {
    config: HighlightConfiguration,
    highlighter: TsHighlighter,
}

impl Highlighter {
    pub fn rust() -> Self {
        let mut config = HighlightConfiguration::new(
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        )
        .expect("bundled Rust queries are valid");
        config.configure(CAPTURES);
        Self { config, highlighter: TsHighlighter::new() }
    }

    /// Highlights the whole document. Re-parses from scratch; incremental
    /// parsing comes later.
    pub fn highlight(&mut self, source: &str) -> Vec<Span> {
        let Ok(events) = self.highlighter.highlight(&self.config, source.as_bytes(), None, |_| None) else {
            return Vec::new();
        };
        let mut spans = Vec::new();
        let mut stack: Vec<Syntax> = Vec::new();
        for event in events {
            match event {
                Ok(HighlightEvent::HighlightStart(h)) => stack.push(syntax_for(CAPTURES[h.0])),
                Ok(HighlightEvent::HighlightEnd) => {
                    stack.pop();
                }
                Ok(HighlightEvent::Source { start, end }) => {
                    if let Some(&syntax) = stack.last()
                        && syntax != Syntax::Plain
                    {
                        spans.push((start..end, syntax));
                    }
                }
                Err(_) => return Vec::new(),
            }
        }
        spans
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
mod tests {
    use super::*;

    #[test]
    fn highlights_keywords_strings_and_comments() {
        let source = "// hi\nfn main() { let s = \"x\"; }";
        let spans = Highlighter::rust().highlight(source);
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
