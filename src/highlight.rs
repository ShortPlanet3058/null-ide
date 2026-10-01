use crate::languages::Language;
use crate::theme::Syntax;
use std::ops::Range;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter as TsHighlighter};

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

pub struct Highlighter {
    config: &'static HighlightConfiguration,
    highlighter: TsHighlighter,
}

impl Highlighter {
    pub fn new(language: &'static Language) -> Option<Self> {
        Some(Self { config: language.highlight_config(CAPTURES)?, highlighter: TsHighlighter::new() })
    }

    /// Highlights the whole document. Re-parses from scratch; incremental
    /// parsing comes later.
    pub fn highlight(&mut self, source: &str) -> Vec<Span> {
        let Ok(events) = self.highlighter.highlight(self.config, source.as_bytes(), None, |_| None) else {
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
        let spans = Highlighter::new(crate::languages::for_path(std::path::Path::new("a.rs")).unwrap())
            .unwrap()
            .highlight(source);
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
        Highlighter::new(language)
            .unwrap()
            .highlight(source)
            .into_iter()
            .map(|(r, s)| (source[r].to_string(), s))
            .collect()
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
