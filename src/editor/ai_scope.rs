//! What ⌘I changes when nothing's selected: the function, class, CSS rule or HTML element
//! the caret's in, as the syntax tree has it, and never more than `MAX_LINES` (a bigger one
//! gives way to the part of it the caret's in). Languages without such units: the
//! paragraph around the caret.

use std::ops::Range;
use tree_sitter::{Node, Point, Tree};

/// Without a selection, ⌘I changes at most this many lines.
pub const MAX_LINES: usize = 40;

/// The kinds of node a language's code is made of, each changed as a whole.
fn units(language: &str) -> &'static [&'static str] {
    match language {
        "Rust" => &[
            "function_item",
            "struct_item",
            "enum_item",
            "union_item",
            "impl_item",
            "trait_item",
            "macro_definition",
            "mod_item",
            "const_item",
            "static_item",
        ],
        "Python" => &["function_definition", "class_definition"],
        "JavaScript" | "TypeScript" | "TSX" => &[
            "function_declaration",
            "generator_function_declaration",
            "function_expression",
            "arrow_function",
            "method_definition",
            "class_declaration",
            "abstract_class_declaration",
            "interface_declaration",
            "enum_declaration",
            "type_alias_declaration",
            "jsx_element",
        ],
        "Java" => &[
            "method_declaration",
            "constructor_declaration",
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "record_declaration",
        ],
        "C#" => &[
            "method_declaration",
            "constructor_declaration",
            "property_declaration",
            "class_declaration",
            "struct_declaration",
            "interface_declaration",
            "enum_declaration",
            "record_declaration",
        ],
        "Go" => &["function_declaration", "method_declaration", "type_declaration", "func_literal"],
        "C" | "C++" => {
            &["function_definition", "struct_specifier", "class_specifier", "enum_specifier", "union_specifier"]
        }
        "Ruby" => &["method", "singleton_method", "class", "module"],
        "PHP" => &[
            "function_definition",
            "method_declaration",
            "class_declaration",
            "interface_declaration",
            "trait_declaration",
        ],
        "Lua" => &["function_declaration", "function_definition"],
        "Shell" => &["function_definition"],
        "HTML" => &["element", "script_element", "style_element"],
        "CSS" => &["rule_set", "media_statement", "keyframes_statement", "supports_statement", "at_rule"],
        "JSON" => &["pair"],
        "YAML" => &["block_mapping_pair", "block_sequence_item"],
        "TOML" => &["table", "table_array_element"],
        _ => &[],
    }
}

fn lines_of(node: Node) -> Range<usize> {
    node.start_position().row..node.end_position().row + 1
}

/// Whether `node` is whole lines: nothing before it on its first, nothing but `;` or `,`
/// after it on its last.
fn whole_lines(node: Node, line: &dyn Fn(usize) -> String) -> bool {
    let (start, end) = (node.start_position(), node.end_position());
    let first = line(start.row);
    let last = line(end.row);
    first.get(..start.column).is_some_and(|before| before.trim().is_empty())
        && last.get(end.column..).is_some_and(|after| after.trim().trim_matches([';', ',']).is_empty())
}

/// The lines ⌘I changes around `caret_line` in `language`'s `tree` (`line` gives a line's
/// text): None when the language has no units, or the caret's in none.
pub fn unit_lines(
    tree: &Tree,
    language: &str,
    caret_line: usize,
    line: &dyn Fn(usize) -> String,
) -> Option<Range<usize>> {
    let units = units(language);
    if units.is_empty() {
        return None;
    }
    // From the caret line's first character.
    let text = line(caret_line);
    let at = Point { row: caret_line, column: text.len() - text.trim_start().len() };
    let mut node = tree.root_node().descendant_for_point_range(at, at)?;
    // The whole-line parts of the unit the caret's in, innermost first.
    let mut parts: Vec<Node> = Vec::new();
    let unit = loop {
        if units.contains(&node.kind()) {
            break node;
        }
        if node.is_named() && whole_lines(node, line) && lines_of(node).contains(&caret_line) {
            parts.push(node);
        }
        // (Not the whole file.)
        match node.parent() {
            Some(parent) if parent.parent().is_some() => node = parent,
            _ => return None,
        }
    };
    let unit = widened(unit);
    let mut lines = lines_of(unit);
    lines.start = notes_above(unit).unwrap_or(lines.start);
    if lines.len() <= MAX_LINES {
        return Some(lines);
    }
    // Too big: the biggest part of it the caret's in that isn't, else the caret's line.
    let part = parts.iter().rev().map(|p| lines_of(*p)).find(|l| l.len() <= MAX_LINES);
    Some(part.unwrap_or(caret_line..caret_line + 1))
}

/// The unit with what it's written in on the same lines (`export`, `const f = …;`, a
/// call it's the callback of) and its decorators.
fn widened(mut node: Node) -> Node {
    while let Some(parent) = node.parent().filter(|p| p.parent().is_some()) {
        let same_lines = parent.start_position().row == node.start_position().row
            && parent.end_position().row == node.end_position().row;
        if !same_lines && parent.kind() != "decorated_definition" {
            break;
        }
        node = parent;
    }
    node
}

/// The first line of the comments and attributes just above `node` (a doc comment, a
/// `#[derive]`), if any.
fn notes_above(node: Node) -> Option<usize> {
    let mut first = None;
    let mut below = node.start_position().row;
    let mut sibling = node.prev_sibling();
    while let Some(above) = sibling {
        let kind = above.kind();
        let note = kind.contains("comment") || kind.contains("attribute") || kind == "decorator";
        if !note || above.end_position().row + 1 < below {
            break;
        }
        below = above.start_position().row;
        first = Some(below);
        sibling = above.prev_sibling();
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_in(language: &str, grammar: tree_sitter::Language, text: &str, caret_line: usize) -> Option<Range<usize>> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar).unwrap();
        let tree = parser.parse(text, None).unwrap();
        let all: Vec<String> = text.lines().map(str::to_string).collect();
        unit_lines(&tree, language, caret_line, &|l| all.get(l).cloned().unwrap_or_default())
    }

    #[test]
    fn html_takes_the_element_the_caret_is_in_not_the_page() {
        let html = "<!doctype html>\n<html>\n<body>\n  <nav>\n    <a href=\"/\">Home</a>\n    <a href=\"/b\">B</a>\n  </nav>\n  <main>\n    <h1>Title</h1>\n  </main>\n</body>\n</html>\n";
        let html_lines = |caret| lines_in("HTML", tree_sitter_html::LANGUAGE.into(), html, caret);
        // A line that's an element: that line.
        assert_eq!(html_lines(4), Some(4..5));
        // Its opening tag's line: the element.
        assert_eq!(html_lines(3), Some(3..7));
        assert_eq!(html_lines(9), Some(7..10));
    }

    #[test]
    fn a_big_unit_gives_way_to_the_part_the_caret_is_in() {
        let filler = |indent: &str| (0..30).map(|i| format!("{indent}x{i} = {i}\n")).collect::<String>();
        let python = format!("def f(x):\n{}    if x:\n        a()\n        b()\n{}", filler("    "), filler("    "));
        let python_lines = |caret| lines_in("Python", tree_sitter_python::LANGUAGE.into(), &python, caret);
        assert_eq!(python_lines(32), Some(31..34));
        // A line straight in it: that line.
        assert_eq!(python_lines(5), Some(5..6));
        let mut html = String::from("<main>\n");
        html.push_str(&(0..50).map(|i| format!("  <p>{i}</p>\n")).collect::<String>());
        html.push_str("  Some text\n</main>\n");
        assert_eq!(lines_in("HTML", tree_sitter_html::LANGUAGE.into(), &html, 51), Some(51..52));
    }

    #[test]
    fn code_takes_the_function_with_its_notes_and_what_it_s_written_in() {
        let rust = "use x;\n\n/// Doubles.\n#[inline]\nfn double(x: i32) -> i32 {\n    let y = x;\n\n    y * 2\n}\n";
        assert_eq!(lines_in("Rust", tree_sitter_rust::LANGUAGE.into(), rust, 7), Some(2..9));
        // Blank lines inside a function don't cut it.
        let python = "import os\n\n@cache\ndef f(x):\n    a = x\n\n    return a\nprint(f(1))\n";
        assert_eq!(lines_in("Python", tree_sitter_python::LANGUAGE.into(), python, 6), Some(2..7));
        let js = "const a = 1;\nexport const f = (x) => {\n  return x;\n};\nf(a);\n";
        assert_eq!(lines_in("JavaScript", tree_sitter_javascript::LANGUAGE.into(), js, 2), Some(1..4));
        // Outside any: none (the paragraph is used).
        assert_eq!(lines_in("JavaScript", tree_sitter_javascript::LANGUAGE.into(), js, 4), None);
        assert_eq!(lines_in("Markdown", tree_sitter_md::LANGUAGE.into(), "# a\n", 0), None);
    }

    #[test]
    fn css_takes_the_rule() {
        let css = "body {\n  margin: 0;\n}\n.a {\n  color: red;\n  padding: 0;\n}\n";
        assert_eq!(lines_in("CSS", tree_sitter_css::LANGUAGE.into(), css, 4), Some(3..7));
    }
}
