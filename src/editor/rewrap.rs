//! Rewrap: a paragraph of comment or text refilled to the line length the project keeps
//! (80 when it says none), its comment markers, quote marks and list indent kept.

use super::{Editor, Selection};
use gpui::{Context, Window, actions};
use std::ops::Range;

actions!(editor, [Rewrap]);

/// What can be rewrapped in a file.
#[derive(Clone, Copy)]
struct Kind {
    /// How the language's line comments start; only comments are rewrapped in code.
    comment: Option<&'static str>,
    /// Markdown or plain text: every paragraph is text.
    prose: bool,
}

/// A line of text to wrap: what comes before the words, and the words.
#[derive(Debug, PartialEq)]
struct Line<'a> {
    /// Indentation, a comment's marker or a quote's `>`, and the spaces after.
    lead: &'a str,
    /// A list item's `-` or `1.` and the spaces after; empty on other lines.
    marker: &'a str,
    words: &'a str,
}

impl Line<'_> {
    /// What the paragraph's following lines start with.
    fn continued(&self) -> String {
        format!("{}{}", self.lead, " ".repeat(self.marker.chars().count()))
    }
}

fn spaces_after(text: &str, at: usize) -> usize {
    at + text[at..].len() - text[at..].trim_start_matches([' ', '\t']).len()
}

/// The line as text to wrap; None for code, blank lines, and lines that are not running
/// text (headings, fences, tables, rules).
fn split(line: &str, kind: Kind) -> Option<Line<'_>> {
    let mut at = spaces_after(line, 0);
    let rest = &line[at..];
    if let Some(comment) = kind.comment.filter(|c| rest.starts_with(c)) {
        at += comment.len();
        // Doc comments: `///`, `//!`, `##`.
        at += line[at..].len() - line[at..].trim_start_matches(|c| comment.contains(c) || c == '!').len();
    } else if kind.comment == Some("//") && (rest.starts_with("* ") || rest == "*") {
        // The middle of a /* … */ block.
        at += 1;
    } else if kind.prose {
        while line[at..].starts_with('>') {
            at = spaces_after(line, at + 1);
        }
    } else {
        return None;
    }
    at = spaces_after(line, at);
    let text = &line[at..];
    let rule = matches!(text.trim_end(), "---" | "***" | "___");
    if text.trim().is_empty() || rule || ["#", "```", "~~~", "|", "<"].iter().any(|s| text.starts_with(s)) {
        return None;
    }
    let digits = text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let marker_len = match text[digits..].chars().next() {
        Some('-' | '*' | '+') if digits == 0 => 1,
        Some('.' | ')') if digits > 0 => digits + 1,
        _ => 0,
    };
    let marker_len =
        if marker_len > 0 && text[marker_len..].starts_with(' ') { spaces_after(text, marker_len) } else { 0 };
    Some(Line { lead: &line[..at], marker: &text[..marker_len], words: &text[marker_len..] })
}

/// Where the paragraph holding line `at` starts.
fn paragraph_start(lines: &[Option<Line>], at: usize) -> usize {
    let mut first = at;
    while let Some(Some(line)) = lines.get(first)
        && line.marker.is_empty()
        && first > 0
        && let Some(above) = &lines[first - 1]
    {
        if above.marker.is_empty() && above.lead == line.lead {
            first -= 1;
        } else if !above.marker.is_empty() && above.continued() == line.lead {
            return first - 1;
        } else {
            break;
        }
    }
    first
}

/// Where the paragraph starting at line `first` ends.
fn paragraph_end(lines: &[Option<Line>], first: usize) -> usize {
    let Some(head) = &lines[first] else { return first + 1 };
    let lead = head.continued();
    let mut end = first + 1;
    while let Some(Some(line)) = lines.get(end)
        && line.marker.is_empty()
        && line.lead == lead
    {
        end += 1;
    }
    end
}

/// How wide `text` shows, tabs going to the next stop.
fn columns(text: &str, tab: usize) -> usize {
    text.chars().fold(0, |col, c| if c == '\t' { (col / tab + 1) * tab } else { col + 1 })
}

/// The paragraph's words refilled into lines no wider than `width` (a longer word gets a
/// line of its own).
fn refill(paragraph: &[&Line], width: usize, tab: usize) -> Vec<String> {
    let first = format!("{}{}", paragraph[0].lead, paragraph[0].marker);
    let next = paragraph[0].continued();
    let mut lines = Vec::new();
    let mut line = first.clone();
    let mut empty = true;
    for word in paragraph.iter().flat_map(|l| l.words.split_whitespace()) {
        if !empty && columns(&line, tab) + 1 + word.chars().count() > width {
            lines.push(std::mem::replace(&mut line, next.clone()));
            empty = true;
        }
        if !empty {
            line.push(' ');
        }
        line.push_str(word);
        empty = false;
    }
    lines.push(line);
    lines
}

/// The paragraphs that `selected` (lines of `text`) touches, each with its lines rewrapped.
fn rewrap(
    text: &[String],
    selected: Range<usize>,
    kind: Kind,
    width: usize,
    tab: usize,
) -> Vec<(Range<usize>, Vec<String>)> {
    // Code between fences (```, ~~~, in Markdown or in doc comments) is never text to wrap.
    let mut fence: Option<String> = None;
    let lines: Vec<Option<Line>> = text
        .iter()
        .map(|l| {
            let words = l.trim_start().trim_start_matches(['/', '!', '#', '>', '*']).trim_start();
            let opens = ["```", "~~~"].into_iter().find(|f| words.starts_with(f));
            match (&fence, opens) {
                (Some(open), Some(f)) if f == open => {
                    fence = None;
                    None
                }
                (Some(_), _) => None,
                (None, Some(f)) => {
                    fence = Some(f.to_string());
                    None
                }
                (None, None) => split(l, kind),
            }
        })
        .collect();
    let mut found = Vec::new();
    let mut at = selected.start;
    while at < selected.end.min(lines.len()) {
        if lines[at].is_none() {
            at += 1;
            continue;
        }
        let first = paragraph_start(&lines, at);
        let end = paragraph_end(&lines, first);
        let paragraph: Vec<&Line> = lines[first..end].iter().flatten().collect();
        found.push((first..end, refill(&paragraph, width, tab)));
        at = end;
    }
    found
}

/// How far around the selection to look for the rest of its paragraphs.
const AROUND: usize = 200;

impl Editor {
    pub(super) fn rewrap(&mut self, _: &Rewrap, _: &mut Window, cx: &mut Context<Self>) {
        self.single_cursor();
        let kind = Kind {
            comment: self.language().and_then(|l| l.line_comment),
            prose: self.is_prose() || self.language().is_none(),
        };
        let width = self.style.ruler.unwrap_or(80);
        let selected = self.selected_lines();
        let window = selected.start.saturating_sub(AROUND)..(selected.end + AROUND).min(self.buffer.len_lines());
        let text = self.line_texts(&window);
        let local = selected.start - window.start..selected.end - window.start;
        let found = rewrap(&text, local, kind, width, super::TAB_SIZE);
        let (Some(first), Some(last)) = (found.first(), found.last()) else {
            let at = self.selection.head;
            return self.show_notice(at, "Rewrap works on comments and text.".into(), cx);
        };
        let span = first.0.start..last.0.end;
        let mut new_lines = Vec::new();
        let mut caret_line = None;
        let caret = self.buffer.point(self.selection.head).0 - window.start;
        let mut at = span.start;
        for (range, lines) in &found {
            new_lines.extend(text[at..range.start].iter().cloned());
            new_lines.extend(lines.iter().cloned());
            if range.contains(&caret) {
                caret_line = Some(new_lines.len() - 1);
            }
            at = range.end;
        }
        if new_lines[..] == text[span.clone()] {
            return;
        }
        let had_selection = !self.selection.is_empty();
        let new_len = new_lines.len();
        let start = window.start + span.start;
        self.rewrite_lines(start..window.start + span.end, new_lines, |p| p, cx);
        let end_of = |this: &Self, line: usize| this.buffer.line_to_char(line) + this.buffer.line_len(line);
        self.selection = if had_selection {
            Selection { anchor: self.buffer.line_to_char(start), head: end_of(self, start + new_len - 1) }
        } else {
            Selection::caret(end_of(self, start + caret_line.unwrap_or(new_len - 1)))
        };
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: Kind = Kind { comment: Some("//"), prose: false };
    const MARKDOWN: Kind = Kind { comment: None, prose: true };

    fn wrap(text: &str, line: usize, kind: Kind, width: usize) -> String {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        let mut out = lines.clone();
        for (range, new) in rewrap(&lines, line..line + 1, kind, width, 4).into_iter().rev() {
            out.splice(range, new);
        }
        out.join("\n")
    }

    #[test]
    fn comments_keep_their_markers_and_indentation() {
        let code = "fn a() {\n    // one two three four five six\n    // seven\n    let x = 1;\n}";
        assert_eq!(
            wrap(code, 2, RUST, 20),
            "fn a() {\n    // one two three\n    // four five six\n    // seven\n    let x = 1;\n}"
        );
        // Doc comments, and the middle of a block comment.
        assert_eq!(wrap("/// a b c d e f", 0, RUST, 9), "/// a b c\n/// d e f");
        assert_eq!(wrap(" * a b c d e f", 0, RUST, 9), " * a b c\n * d e f");
        // Short lines are filled up again; code is left alone.
        assert_eq!(wrap("// a\n// b\n// c", 0, RUST, 80), "// a b c");
        assert_eq!(wrap("let a = b * c;", 0, RUST, 5), "let a = b * c;");
    }

    #[test]
    fn markdown_paragraphs_lists_and_quotes() {
        let md = "# Title\n\nSome words that run\non.\n\n- an item that is long\n  and goes on\n- next";
        assert_eq!(
            wrap(md, 3, MARKDOWN, 80),
            "# Title\n\nSome words that run on.\n\n- an item that is long\n  and goes on\n- next"
        );
        assert_eq!(
            wrap(md, 6, MARKDOWN, 14),
            "# Title\n\nSome words that run\non.\n\n- an item that\n  is long and\n  goes on\n- next"
        );
        assert_eq!(wrap("> quoted words here", 0, MARKDOWN, 12), "> quoted\n> words here");
        assert_eq!(wrap("10. ten tens", 0, MARKDOWN, 8), "10. ten\n    tens");
        // Headings, fences and tables aren't paragraphs.
        assert_eq!(wrap("# a b c d", 0, MARKDOWN, 3), "# a b c d");
        assert_eq!(wrap("| a | b |", 0, MARKDOWN, 3), "| a | b |");
    }

    /// In an editor: the caret's paragraph, the caret at its end; a selection's paragraphs,
    /// still selected; ⌘Z.
    #[gpui::test]
    fn rewraps_in_the_editor(cx: &mut gpui::TestAppContext) {
        use gpui::Focusable;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let text = format!("{}\n\nshort\nlines\n", ["word"; 20].join(" "));
        let (e, cx) = cx
            .add_window_view(|_, cx| Editor::new(crate::buffer::Buffer::from_text(&text), Some("notes.md".into()), cx));
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            e.selection = Selection::caret(3);
        });
        cx.dispatch_action(Rewrap);
        let first = format!("{}\n{}", ["word"; 16].join(" "), ["word"; 4].join(" "));
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), format!("{first}\n\nshort\nlines\n"));
            assert_eq!(e.selection.head, first.chars().count());
        });
        e.update(cx, |e, _| e.selection = Selection { anchor: 0, head: e.buffer.len_chars() });
        cx.dispatch_action(Rewrap);
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), format!("{first}\n\nshort lines\n"));
            assert_eq!(e.buffer.slice(e.selection.range()), format!("{first}\n\nshort lines"));
        });
        cx.simulate_keystrokes("cmd-z cmd-z");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), text));
    }

    #[test]
    fn code_in_fences_is_left_alone() {
        let md = "Some words\n\n```sh\ncargo build\ncargo run\n```\n";
        assert_eq!(wrap(md, 3, MARKDOWN, 80), md.trim_end());
        let doc = "/// ```\n/// let x = 1;\n/// let y = 2;\n/// ```";
        assert_eq!(wrap(doc, 1, RUST, 80), doc);
    }

    #[test]
    fn a_long_word_gets_its_own_line() {
        assert_eq!(wrap("a https://example.com/long b", 0, MARKDOWN, 10), "a\nhttps://example.com/long\nb");
    }
}
