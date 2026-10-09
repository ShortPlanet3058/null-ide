//! A file's outline, for the sidebar's Outline view: its functions and types (a Markdown
//! file's headings), each as deep as it's written inside another, and which one the caret
//! is in.

use std::path::Path;

/// A function, type or heading, and how deep it sits (0: at the top).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    /// The word that defines it (`fn`, `class`), or the heading's `##`.
    pub kind: &'static str,
    /// Zero-based line.
    pub row: usize,
    pub depth: usize,
}

/// How far `line` is indented, a tab counting as four spaces.
fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

/// The outline of `text` (the file at `path`): what ⌘⇧O lists, nested by indentation (a
/// method inside its class), or for Markdown by the headings' levels. An item is left at
/// the first line after it indented no deeper (its closing `}`, the next statement), so
/// what follows isn't taken to be inside it.
pub fn outline(path: &Path, text: &str) -> Vec<Item> {
    let mut definitions = crate::project_index::definitions_in(path, text).into_iter().peekable();
    let mut items = Vec::new();
    // The indents of the items it's inside, deepest last.
    let mut around: Vec<usize> = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let Some(d) = definitions.next_if(|d| d.row == row) else {
            if !line.trim().is_empty() && !around.is_empty() {
                let indent = indent_of(line);
                while around.last().is_some_and(|&a| a >= indent) {
                    around.pop();
                }
            }
            continue;
        };
        let depth = if d.kind.starts_with('#') {
            d.kind.len() - 1
        } else {
            let indent = indent_of(line);
            while around.last().is_some_and(|&a| a >= indent) {
                around.pop();
            }
            around.push(indent);
            around.len() - 1
        };
        items.push(Item { name: d.name, kind: d.kind, row: d.row, depth });
    }
    items
}

/// The item the caret on `row` is in: the last one starting at or above it.
pub fn current(items: &[Item], row: usize) -> Option<usize> {
    items.iter().rposition(|item| item.row <= row)
}

/// Where the caret on `row` is, from the outside in (`Shop`, `checkout`): the item it's
/// in, and each one that item sits inside.
#[cfg(test)]
pub fn trail(items: &[Item], row: usize) -> Vec<&str> {
    trail_items(items, row).into_iter().map(|item| item.name.as_str()).collect()
}

/// The items of `trail`, outermost first (each with its row, to go to).
pub fn trail_items(items: &[Item], row: usize) -> Vec<&Item> {
    let Some(at) = current(items, row) else { return Vec::new() };
    let mut trail = vec![&items[at]];
    let mut depth = items[at].depth;
    for item in items[..at].iter().rev() {
        if item.depth < depth {
            trail.push(item);
            depth = item.depth;
        }
    }
    trail.reverse();
    trail
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short(items: &[Item]) -> Vec<(&str, usize, usize)> {
        items.iter().map(|i| (i.name.as_str(), i.row, i.depth)).collect()
    }

    #[test]
    fn methods_sit_inside_their_types() {
        let python = "class Shop:\n    def open(self):\n        pass\n\n    def close(self):\n        pass\n\ndef main():\n    pass\n";
        let items = outline(Path::new("shop.py"), python);
        assert_eq!(short(&items), [("Shop", 0, 0), ("open", 1, 1), ("close", 4, 1), ("main", 7, 0)]);
        assert_eq!(current(&items, 2), Some(1), "in open");
        assert_eq!(current(&items, 5), Some(2), "in close");
        assert_eq!(current(&items, 8), Some(3));
        assert_eq!(trail(&items, 5), ["Shop", "close"]);
        assert_eq!(trail(&items, 8), ["main"]);
    }

    /// Rust's methods sit in an `impl`, which isn't an item: not inside what came before it.
    #[test]
    fn what_ends_isn_t_a_parent() {
        let rust = "fn helper() {}\n\nstruct Shop;\n\nimpl Shop {\n    fn open(&self) {}\n}\n\nfn main() {\n    let x = 1;\n}\n";
        let items = outline(Path::new("shop.rs"), rust);
        assert_eq!(short(&items), [("helper", 0, 0), ("Shop", 2, 0), ("open", 5, 0), ("main", 8, 0)]);
        assert_eq!(trail(&items, 5), ["open"]);
        // A class's own lines between its methods don't end it.
        let python = "class A:\n    x = 1\n    def f(self):\n        pass\n";
        assert_eq!(short(&outline(Path::new("a.py"), python)), [("A", 0, 0), ("f", 2, 1)]);
    }

    #[test]
    fn headings_by_their_level() {
        let markdown = "# Guide\n\nIntro.\n\n## Install\n\n```sh\n# not a heading\n```\n\n### macOS\n\n## Use\n";
        let items = outline(Path::new("guide.md"), markdown);
        assert_eq!(short(&items), [("Guide", 0, 0), ("Install", 4, 1), ("macOS", 10, 2), ("Use", 12, 1)]);
        assert_eq!(current(&items, 2), Some(0));
    }

    #[test]
    fn nothing_above_the_first() {
        let items = outline(Path::new("a.rs"), "// notes\n\nfn main() {}\n");
        assert_eq!(current(&items, 0), None);
        assert_eq!(current(&items, 2), Some(0));
        assert!(outline(Path::new("a.txt"), "just words\n").is_empty());
    }
}
