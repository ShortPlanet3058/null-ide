//! Pasted code takes the indentation of where it goes, its lines keeping theirs relative
//! to each other. ⌥⇧⌘V pastes it as it was.

use crate::file_style::Indent;

/// How wide the indentation at the start of `line` shows.
pub(super) fn indent_columns(line: &str, tab: usize) -> usize {
    line.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .fold(0, |col, c| if c == '\t' { (col / tab + 1) * tab } else { col + 1 })
}

/// `columns` of indentation, written the file's way.
fn indentation(columns: usize, unit: Indent, tab: usize) -> String {
    match unit {
        Indent::Tabs => format!("{}{}", "\t".repeat(columns / tab), " ".repeat(columns % tab)),
        Indent::Spaces(_) => " ".repeat(columns),
    }
}

/// `text` with its lines moved from `base` columns of indentation to `target`, each keeping
/// how much deeper or shallower than `base` it was. The first line is left as it is when
/// `first` is false (it goes in after other text on the line). Blank lines stay blank.
pub(super) fn reindent(text: &str, base: usize, target: usize, first: bool, unit: Indent, tab: usize) -> String {
    text.split('\n')
        .enumerate()
        .map(|(i, line)| {
            if i == 0 && !first {
                return line.to_string();
            }
            let words = line.trim_start_matches([' ', '\t']);
            if words.trim().is_empty() {
                return if line.ends_with('\r') { "\r".into() } else { String::new() };
            }
            let columns = (target + indent_columns(line, tab)).saturating_sub(base);
            format!("{}{words}", indentation(columns, unit, tab))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACES: Indent = Indent::Spaces(4);

    #[test]
    fn lines_move_together_to_the_new_depth() {
        let copied = "    if a {\n        b();\n    }\n";
        assert_eq!(reindent(copied, 4, 8, true, SPACES, 4), "        if a {\n            b();\n        }\n");
        assert_eq!(reindent(copied, 4, 0, true, SPACES, 4), "if a {\n    b();\n}\n");
        // Into a file indented with tabs.
        assert_eq!(reindent(copied, 4, 4, true, Indent::Tabs, 4), "\tif a {\n\t\tb();\n\t}\n");
    }

    #[test]
    fn after_text_on_the_line_only_the_lines_below_move() {
        // `foo(` copied from a line indented 8, pasted on a line indented 4.
        let copied = "foo(\n            a,\n        )";
        assert_eq!(reindent(copied, 8, 4, false, SPACES, 4), "foo(\n        a,\n    )");
    }

    /// Copied and pasted in a Rust file: a line into a deeper block, a block out to the top
    /// level; ⌥⇧⌘V puts it in as it was.
    #[gpui::test]
    fn pasted_code_takes_the_indentation_where_it_goes(cx: &mut gpui::TestAppContext) {
        use crate::editor::{Editor, Selection};
        use gpui::Focusable;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let code = "fn a() {\n    b();\n}\nfn c() {\n    if x {\n        y();\n    }\n}\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(crate::buffer::Buffer::from_text(code), Some("x.rs".into()), cx));
        let at = |e: &Editor, line: usize, column: usize| e.buffer.line_to_char(line) + column;
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            e.selection = Selection::caret(at(e, 1, 6));
        });
        // The whole line, with nothing selected, into the `if`.
        cx.simulate_keystrokes("cmd-c");
        e.update(cx, |e, _| e.selection = Selection::caret(at(e, 5, 0)));
        cx.simulate_keystrokes("cmd-v");
        e.update(cx, |e, _| assert_eq!(e.buffer.line_text(5), "        b();"));
        // The `if` block, out to the end of the file.
        e.update(cx, |e, _| e.selection = Selection { anchor: at(e, 4, 0), head: at(e, 8, 0) });
        cx.simulate_keystrokes("cmd-c");
        e.update(cx, |e, _| e.selection = Selection::caret(e.buffer.len_chars()));
        cx.simulate_keystrokes("cmd-v");
        let end = "}\nif x {\n    b();\n    y();\n}\n";
        e.update(cx, |e, _| assert!(e.buffer.to_string().ends_with(end), "{}", e.buffer.to_string()));
        cx.simulate_keystrokes("cmd-z alt-shift-cmd-v");
        let end = "}\n    if x {\n        b();\n        y();\n    }\n";
        e.update(cx, |e, _| assert!(e.buffer.to_string().ends_with(end), "{}", e.buffer.to_string()));
    }

    /// Python has no `}` to say a block ended: pasted between two functions or above one,
    /// code stays at the top level. Above a `}`, it goes inside the block.
    #[gpui::test]
    fn pasted_lines_go_where_the_line_says(cx: &mut gpui::TestAppContext) {
        use crate::editor::{Editor, Selection};
        use gpui::Focusable;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let code = "def a():\n    return 1\n\ndef b():\n    pass\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(crate::buffer::Buffer::from_text(code), Some("x.py".into()), cx));
        let at = |e: &Editor, line: usize| e.buffer.line_to_char(line);
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            e.selection = Selection { anchor: at(e, 3), head: at(e, 5) };
        });
        cx.simulate_keystrokes("cmd-c");
        // On the blank line between the functions.
        e.update(cx, |e, _| e.selection = Selection::caret(at(e, 2)));
        cx.simulate_keystrokes("cmd-v");
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "def a():\n    return 1\ndef b():\n    pass\n\ndef b():\n    pass\n")
        });
        // Above a top-level `def`, as whole lines.
        e.update(cx, |e, _| e.selection = Selection::caret(at(e, 2)));
        cx.simulate_keystrokes("cmd-v");
        e.update(cx, |e, _| assert_eq!(e.buffer.line_text(2), "def b():"));

        let code = "fn a() {\n    x();\n}\n// y();\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(crate::buffer::Buffer::from_text(code), Some("x.rs".into()), cx));
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            e.selection = Selection::caret(at(e, 3) + 3);
        });
        cx.simulate_keystrokes("cmd-c");
        e.update(cx, |e, _| e.selection = Selection::caret(at(e, 2)));
        cx.simulate_keystrokes("cmd-v");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "fn a() {\n    x();\n    // y();\n}\n// y();\n"));
    }

    #[test]
    fn blank_lines_stay_blank_and_nothing_goes_below_zero() {
        assert_eq!(reindent("  a\n\n    \nb", 2, 0, true, SPACES, 4), "a\n\n\nb");
        assert_eq!(reindent("a\r\n  \r\n  b\r\n", 0, 4, true, SPACES, 4), "    a\r\n\r\n      b\r\n");
    }
}
