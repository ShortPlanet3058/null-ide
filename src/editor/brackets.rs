//! Bracket pairs in colour (a setting): each bracket of the code, not those in strings or
//! comments, coloured by how deep it is, so a pair reads at a glance.

use super::Editor;
use crate::highlight::Span;
use crate::theme::Syntax;
use std::rc::Rc;
use tree_sitter::Tree;

/// Files longer than this are left as they are: the whole tree is walked after each edit,
/// a few milliseconds at this size.
const LONGEST: usize = 500_000;

/// What opens a pair, and the closing bracket it waits for (`${` in a template string
/// closes with `}`).
fn closer_of(kind: &str) -> Option<&'static str> {
    match kind {
        "(" => Some(")"),
        "[" => Some("]"),
        "{" | "${" | "#{" => Some("}"),
        _ => None,
    }
}

/// The brackets in `tree`, in order, each coloured by its depth: a pair the same colour.
/// One closing nothing open is left as it is. Brackets in strings and comments aren't
/// tokens of the tree, so they're left out by themselves.
pub(crate) fn coloured(tree: &Tree) -> Vec<Span> {
    let mut out = Vec::new();
    // The brackets open, by what closes them.
    let mut open: Vec<&'static str> = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        // One the parser put in where it's missing (`g(a;`): it closes its bracket, unseen,
        // so what follows isn't taken a level deeper.
        if node.is_missing() && open.last() == Some(&node.kind()) {
            open.pop();
        } else if node.child_count() == 0 && !node.is_missing() {
            let kind = node.kind();
            let range = node.start_byte()..node.end_byte();
            if let Some(closer) = closer_of(kind) {
                out.push((range, Syntax::Bracket((open.len() % 3) as u8)));
                open.push(closer);
            } else if matches!(kind, ")" | "]" | "}") && open.last() == Some(&kind) {
                open.pop();
                out.push((range, Syntax::Bracket((open.len() % 3) as u8)));
            }
        }
        if cursor.goto_first_child() || cursor.goto_next_sibling() {
            continue;
        }
        loop {
            if !cursor.goto_parent() {
                break 'walk;
            }
            if cursor.goto_next_sibling() {
                break;
            }
        }
    }
    out
}

impl Editor {
    /// The code's brackets in colour, worked out again only when the text changes; None
    /// without a syntax tree up to date, or for a very long file.
    fn bracket_spans(&self) -> Option<Rc<Vec<Span>>> {
        let revision = self.buffer.revision();
        if let Some((r, spans)) = &*self.brackets.borrow()
            && *r == revision
        {
            return Some(spans.clone());
        }
        let highlighter = self.highlighter.as_ref()?;
        if highlighter.revision() != revision || self.buffer.rope().len_bytes() > LONGEST {
            return None;
        }
        let spans = Rc::new(coloured(highlighter.tree()?));
        *self.brackets.borrow_mut() = Some((revision, spans.clone()));
        Some(spans)
    }

    /// The colours to draw for the bytes `shown`: the grammar's, the server's over them,
    /// and with coloured brackets on, the brackets'. None when the grammar's alone will do.
    /// A line's colours to draw on its own (a pinned line of sticky scroll): as
    /// `spans_to_draw` has them for the rest.
    pub fn line_spans_to_draw(&mut self, line: usize, brackets: bool) -> Vec<Span> {
        let grammar = self.line_spans(line);
        let spans = self.line_spans_with_meaning(line, grammar);
        let Some(all) = brackets.then(|| self.bracket_spans()).flatten() else { return spans };
        let shown = self.buffer.line_to_byte(line)..self.buffer.line_to_byte(line + 1);
        let from = all.partition_point(|(r, _)| r.end <= shown.start);
        let to = from + all[from..].partition_point(|(r, _)| r.start < shown.end);
        if from == to { spans } else { super::meaning::overlay(&spans, &all[from..to]) }
    }

    pub fn spans_to_draw(&self, shown: std::ops::Range<usize>, brackets: bool) -> Option<Vec<Span>> {
        let meant = self.spans_with_meaning(shown.clone());
        let Some(all) = brackets.then(|| self.bracket_spans()).flatten() else { return meant };
        let from = all.partition_point(|(r, _)| r.end <= shown.start);
        let to = from + all[from..].partition_point(|(r, _)| r.start < shown.end);
        if from == to {
            return meant;
        }
        let first = self.spans.partition_point(|(r, _)| r.end <= shown.start);
        let last = first + self.spans[first..].partition_point(|(r, _)| r.start < shown.end);
        let base = meant.as_deref().unwrap_or(&self.spans[first..last]);
        Some(super::meaning::overlay(base, &all[from..to]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depths(language: &str, text: &str) -> Vec<(String, u8)> {
        let language = crate::languages::all().iter().find(|l| l.name == language).expect("a grammar");
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language.grammar()).unwrap();
        let tree = parser.parse(text, None).unwrap();
        coloured(&tree)
            .into_iter()
            .map(|(r, s)| match s {
                Syntax::Bracket(d) => (text[r].to_string(), d),
                _ => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn pairs_share_a_colour_and_strings_hold_none() {
        let got = depths("Rust", "fn f(a: [u8; 2]) { g(\"(\") } // )\n");
        let want = [("(", 0), ("[", 1), ("]", 1), (")", 0), ("{", 0), ("(", 1), (")", 1), ("}", 0)];
        assert_eq!(got, want.map(|(b, d)| (b.to_string(), d)));
        // Deeper than three: the colours come round again.
        // A closing bracket missing: what follows keeps its depth.
        let mended = depths("Rust", "fn f() { g(a; }\nfn h() {}\n");
        assert_eq!(mended.last(), Some(&("}".to_string(), 0)), "{mended:?}");
        let deep = depths("Rust", "fn f() { ((((1)))) }\n");
        assert_eq!(deep.iter().map(|(_, d)| *d).collect::<Vec<_>>(), [0, 0, 0, 1, 2, 0, 1, 1, 0, 2, 1, 0]);
    }

    #[test]
    fn a_template_string_s_part_pairs_with_its_brace() {
        let got = depths("JavaScript", "const s = `a${f(x)}b`;\n");
        let want = [("${", 0), ("(", 1), (")", 1), ("}", 0)];
        assert_eq!(got, want.map(|(b, d)| (b.to_string(), d)));
    }
}
