//! Commands that work with the shape of the code: growing the selection to the
//! enclosing piece of syntax and back, jumping between brackets, starting a line above
//! or below, joining lines, sorting them, and changing case.

use super::{EditKind, Editor, Selection};
use gpui::{App, Context, KeyBinding, Window, actions};
use std::ops::Range;

actions!(
    structure,
    [
        ExpandSelection,
        ShrinkSelection,
        GoToMatchingBracket,
        NewlineBelow,
        NewlineAbove,
        JoinLines,
        SortLines,
        ReverseLines,
        RemoveDuplicateLines,
        UpperCase,
        LowerCase,
        SnakeCase,
        CamelCase,
        PascalCase,
        KebabCase,
        TitleCase,
        NextChange,
        PreviousChange,
        TrimTrailingWhitespace
    ]
);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Editor");
    let (expand, shrink) = if cfg!(target_os = "macos") {
        ("ctrl-shift-cmd-right", "ctrl-shift-cmd-left")
    } else {
        ("alt-shift-right", "alt-shift-left")
    };
    cx.bind_keys([
        KeyBinding::new(expand, ExpandSelection, ctx),
        KeyBinding::new(shrink, ShrinkSelection, ctx),
        KeyBinding::new("secondary-shift-\\", GoToMatchingBracket, ctx),
        KeyBinding::new("secondary-enter", NewlineBelow, ctx),
        KeyBinding::new("secondary-shift-enter", NewlineAbove, ctx),
        KeyBinding::new("ctrl-j", JoinLines, ctx),
        KeyBinding::new("alt-f5", NextChange, ctx),
        KeyBinding::new("alt-shift-f5", PreviousChange, ctx),
    ]);
}

/// The words a name is made of: split at `_`, `-`, spaces and dots, and where the case
/// changes (`parseHTTPResponse` → parse, HTTP, Response).
fn name_words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut word = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if matches!(c, '_' | '-' | '.') || c.is_whitespace() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        let before = i.checked_sub(1).map(|j| chars[j]);
        let after = chars.get(i + 1);
        let starts = c.is_uppercase()
            && (before.is_some_and(|b| b.is_lowercase() || b.is_ascii_digit())
                || before.is_some_and(char::is_uppercase) && after.is_some_and(|a| a.is_lowercase()));
        if starts && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.push(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn lower(word: &str) -> String {
    word.to_lowercase()
}

fn capital(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map_or_else(String::new, |first| first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect())
}

fn join_words(words: &[String], between: &str, each: fn(&str) -> String) -> String {
    words.iter().map(|w| each(w)).collect::<Vec<_>>().join(between)
}

/// A name style applied to each line on its own: its line breaks ("\n", "\r\n" or "\r")
/// and the spaces around its words stay as they are.
fn by_line(text: &str, style: impl Fn(&[String]) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(['\n', '\r']).unwrap_or(rest.len());
        let (line, after) = rest.split_at(end);
        let words = line.trim();
        if words.is_empty() {
            out.push_str(line);
        } else {
            let lead = &line[..line.len() - line.trim_start().len()];
            let trail = &line[line.trim_end().len()..];
            out.push_str(lead);
            out.push_str(&style(&name_words(words)));
            out.push_str(trail);
        }
        let brk = if after.starts_with("\r\n") { 2 } else { usize::from(!after.is_empty()) };
        out.push_str(&after[..brk]);
        rest = &after[brk..];
    }
    out
}

/// Selections grown by [`ExpandSelection`], to shrink back through.
#[derive(Default)]
pub(super) struct Expansions {
    /// The selection before each step, last step last.
    before: Vec<Selection>,
    /// Where the last step left the selection: if it moved since, the steps are forgotten.
    at: Option<Selection>,
}

/// The smallest range in `ranges` (innermost first) that holds `selection` and is larger.
fn next_larger(selection: &Range<usize>, ranges: impl Iterator<Item = Range<usize>>) -> Option<Range<usize>> {
    ranges
        .filter(|r| r.start <= selection.start && selection.end <= r.end)
        .find(|r| r.start < selection.start || selection.end < r.end)
}

impl Editor {
    pub(super) fn expand_selection(&mut self, _: &ExpandSelection, _: &mut Window, cx: &mut Context<Self>) {
        self.single_cursor();
        let current = self.selection;
        let range = current.range();
        let Some(larger) = self.syntax_parent(&range).or_else(|| self.plain_parent(&range)) else { return };
        if self.expansions.at != Some(current) {
            self.expansions.before.clear();
        }
        self.expansions.before.push(current);
        self.selection = Selection { anchor: larger.start, head: larger.end };
        self.expansions.at = Some(self.selection);
        self.goal_column = None;
        self.touch(cx);
    }

    pub(super) fn shrink_selection(&mut self, _: &ShrinkSelection, _: &mut Window, cx: &mut Context<Self>) {
        if self.expansions.at != Some(self.selection) {
            self.expansions.before.clear();
            return;
        }
        let Some(previous) = self.expansions.before.pop() else { return };
        self.selection = previous;
        self.expansions.at = (!self.expansions.before.is_empty()).then_some(previous);
        self.goal_column = None;
        self.touch(cx);
    }

    /// The piece of syntax just around `range` (chars), from the file's syntax tree.
    fn syntax_parent(&mut self, range: &Range<usize>) -> Option<Range<usize>> {
        let highlighter = self.highlighter.as_mut()?;
        highlighter.sync(&self.buffer);
        let tree = highlighter.tree()?;
        let rope = self.buffer.rope();
        let bytes = rope.char_to_byte(range.start)..rope.char_to_byte(range.end);
        let mut node = tree.root_node().descendant_for_byte_range(bytes.start, bytes.end)?;
        let mut ranges = Vec::new();
        loop {
            ranges.push(node.start_byte()..node.end_byte());
            let Some(parent) = node.parent() else { break };
            node = parent;
        }
        let larger = next_larger(&bytes, ranges.into_iter())?;
        Some(rope.byte_to_char(larger.start)..rope.byte_to_char(larger.end))
    }

    /// Without a syntax tree: the word, then the line, then everything.
    fn plain_parent(&self, range: &Range<usize>) -> Option<Range<usize>> {
        let word = self.word_at(range.start);
        let first = self.buffer.point(range.start).0;
        let last = self.buffer.point(range.end).0;
        let line = self.buffer.line_to_char(first)..self.buffer.line_to_char(last) + self.buffer.line_len(last);
        let all = 0..self.buffer.len_chars();
        next_larger(range, [word, line, all].into_iter())
    }

    pub(super) fn next_change(&mut self, _: &NextChange, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_change(true, cx);
    }

    pub(super) fn previous_change(&mut self, _: &PreviousChange, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_change(false, cx);
    }

    /// ⌥F5: to the next block of lines changed since the last commit (round to the first).
    fn go_to_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        let line = self.buffer.point(self.selection.head).0;
        let starts: Vec<usize> = self.git_hunks.iter().map(|h| h.lines.start).collect();
        let target = if forward {
            starts.iter().find(|&&s| s > line).or(starts.first())
        } else {
            starts.iter().rev().find(|&&s| s < line).or(starts.last())
        };
        let Some(&target) = target else {
            let at = self.selection.head;
            return self.show_notice(at, "Nothing changed since the last commit.".into(), cx);
        };
        cx.emit(super::EditorEvent::Jumped { from: self.caret_point() });
        let target = target.min(self.buffer.len_lines().saturating_sub(1));
        let indent = self.buffer.line_text(target).chars().take_while(|c| c.is_whitespace()).count();
        self.single_cursor();
        self.selection = Selection::caret(self.buffer.offset(target, indent));
        self.goal_column = None;
        self.touch(cx);
    }

    /// ⌘⇧\: to the bracket matching the one at the caret, or to the opening one around it.
    pub(super) fn go_to_matching_bracket(&mut self, _: &GoToMatchingBracket, _: &mut Window, cx: &mut Context<Self>) {
        self.single_cursor();
        let target = match self.matching_brackets() {
            Some((at, other)) => {
                let head = self.selection.head;
                // From either side of one bracket to the same side of the other.
                if head == at + 1 { other + 1 } else { other }
            }
            None => match self.enclosing_open_bracket() {
                Some(open) => open,
                None => return,
            },
        };
        self.selection = Selection::caret(target);
        self.goal_column = None;
        self.touch(cx);
    }

    fn enclosing_open_bracket(&self) -> Option<usize> {
        const LIMIT: usize = 20_000;
        let mut chars = self.buffer.rope().chars_at(self.selection.head);
        let mut depth = 0i32;
        let mut i = self.selection.head;
        for _ in 0..LIMIT {
            let c = chars.prev()?;
            i -= 1;
            match c {
                ')' | ']' | '}' => depth += 1,
                '(' | '[' | '{' if depth == 0 => return Some(i),
                '(' | '[' | '{' => depth -= 1,
                _ => {}
            }
        }
        None
    }

    /// The indentation of `line`, plus one level if it opens a block.
    pub(super) fn indent_after(&self, line: usize) -> String {
        let text = self.buffer.line_text(line);
        let indent: String = text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let trimmed = text.trim_end();
        let opens = trimmed.ends_with(['{', '(', '[']) || (trimmed.ends_with(':') && self.language_name() == "Python");
        if opens { format!("{indent}{}", self.style.indent.unit()) } else { indent }
    }

    /// ⌘↵: a new line below this one, wherever the caret is in it.
    pub(super) fn newline_below(&mut self, _: &NewlineBelow, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            let line = this.buffer.point(this.selection.head).0;
            let end = this.buffer.line_to_char(line) + this.buffer.line_len(line);
            let text = format!("{}{}", this.style.line_ending.text(), this.indent_after(line));
            this.edit(end..end, &text, EditKind::Other, cx);
            this.selection = Selection::caret(end + text.chars().count());
        });
        self.touch(cx);
    }

    /// ⌘⇧↵: a new line above this one.
    pub(super) fn newline_above(&mut self, _: &NewlineAbove, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            let line = this.buffer.point(this.selection.head).0;
            let start = this.buffer.line_to_char(line);
            let text = this.buffer.line_text(line);
            let indent: String = text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            let insert = format!("{indent}{}", this.style.line_ending.text());
            this.edit(start..start, &insert, EditKind::Other, cx);
            this.selection = Selection::caret(start + indent.chars().count());
        });
        self.touch(cx);
    }

    /// ⌃J: the next line joins this one (or the selected lines join into one), with one
    /// space between and the indentation dropped.
    pub(super) fn join_lines(&mut self, _: &JoinLines, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            let mut lines = this.selected_lines();
            if lines.len() < 2 {
                lines.end = (lines.start + 2).min(this.buffer.len_lines());
            }
            if lines.len() < 2 {
                return;
            }
            let texts: Vec<String> = lines.clone().map(|l| this.buffer.line_text(l)).collect();
            let (joined, caret) = join(&texts);
            let start = this.buffer.line_to_char(lines.start);
            let last = lines.end - 1;
            let end = this.buffer.line_to_char(last) + this.buffer.line_len(last);
            this.edit(start..end, &joined, EditKind::Other, cx);
            this.selection = Selection::caret(start + caret);
        });
        self.touch(cx);
    }

    /// All the file's lines, but the empty one after its last line break (that break is the
    /// last line's own).
    fn all_lines(&self) -> Range<usize> {
        let count = self.buffer.len_lines();
        if count > 1 && self.buffer.line_len(count - 1) == 0 { 0..count - 1 } else { 0..count }
    }

    /// Spaces and tabs at the ends of the file's lines, gone (in one undo); the caret stays
    /// on its line.
    pub(super) fn trim_trailing_whitespace(
        &mut self,
        _: &TrimTrailingWhitespace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lines = self.all_lines();
        let texts = self.line_texts(&lines);
        let trimmed: Vec<String> = texts.iter().map(|t| t.trim_end_matches([' ', '\t']).to_string()).collect();
        if trimmed == texts {
            return;
        }
        let lengths: Vec<usize> = trimmed.iter().map(|t| t.chars().count()).collect();
        self.rewrite_lines(lines, trimmed, move |(l, c)| (l, c.min(lengths.get(l).copied().unwrap_or(c))), cx);
    }

    /// The file's indentation, `from` levels made `to` ones (2 spaces to 4, spaces to
    /// tabs), in one undo; what's left over (a space or two lining something up) stays.
    pub(crate) fn reindent_file(
        &mut self,
        from: crate::file_style::Indent,
        to: crate::file_style::Indent,
        cx: &mut Context<Self>,
    ) {
        let lines = self.all_lines();
        let texts = self.line_texts(&lines);
        let mut moved: Vec<(usize, usize)> = Vec::with_capacity(texts.len());
        let new: Vec<String> = texts
            .iter()
            .map(|text| {
                let (line, old, new) = reindented(text, from, to);
                moved.push((old, new));
                line
            })
            .collect();
        if new == texts {
            return;
        }
        // The caret keeps its place in the text after the indentation (or in it, as far in).
        let place = move |(l, c): (usize, usize)| match moved.get(l) {
            Some(&(old, new)) if c >= old => (l, c - old + new),
            Some(&(_, new)) => (l, c.min(new)),
            None => (l, c),
        };
        self.rewrite_lines(lines, new, place, cx);
    }

    /// The selected lines, in order (letters before case, then as written).
    pub(super) fn sort_lines(&mut self, _: &SortLines, _: &mut Window, cx: &mut Context<Self>) {
        self.reorder_lines(
            |mut texts| {
                texts.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
                texts
            },
            "sort",
            cx,
        );
    }

    /// The selected lines, last first.
    pub(super) fn reverse_lines(&mut self, _: &ReverseLines, _: &mut Window, cx: &mut Context<Self>) {
        self.reorder_lines(
            |mut texts| {
                texts.reverse();
                texts
            },
            "reverse",
            cx,
        );
    }

    /// Of the selected lines, each one only once (where it first is).
    pub(super) fn remove_duplicate_lines(&mut self, _: &RemoveDuplicateLines, _: &mut Window, cx: &mut Context<Self>) {
        self.reorder_lines(
            |texts| {
                let mut seen = std::collections::HashSet::new();
                texts.into_iter().filter(|t| seen.insert(t.clone())).collect()
            },
            "go through",
            cx,
        );
    }

    /// The selected lines rewritten by `change` (which may drop some), still selected.
    fn reorder_lines(&mut self, change: impl FnOnce(Vec<String>) -> Vec<String>, verb: &str, cx: &mut Context<Self>) {
        self.single_cursor();
        let lines = self.selected_lines();
        if lines.len() < 2 {
            let at = self.selection.head;
            return self.show_notice(at, format!("Select the lines to {verb}."), cx);
        }
        let texts = change(self.line_texts(&lines));
        let first = lines.start;
        let count = texts.len().max(1);
        let last_len = texts.last().map_or(0, |t| t.chars().count());
        self.rewrite_lines(lines, texts, |p| p, cx);
        // The new lines stay selected.
        let last = first + count - 1;
        let start = self.buffer.line_to_char(first);
        let end = self.buffer.line_to_char(last) + last_len.min(self.buffer.line_len(last));
        self.selection = Selection { anchor: start, head: end };
        cx.notify();
    }

    pub(super) fn upper_case(&mut self, _: &UpperCase, _: &mut Window, cx: &mut Context<Self>) {
        self.change_case(str::to_uppercase, cx);
    }

    pub(super) fn lower_case(&mut self, _: &LowerCase, _: &mut Window, cx: &mut Context<Self>) {
        self.change_case(str::to_lowercase, cx);
    }

    pub(super) fn snake_case(&mut self, _: &SnakeCase, _: &mut Window, cx: &mut Context<Self>) {
        self.change_case(|t| by_line(t, |w| join_words(w, "_", lower)), cx);
    }

    pub(super) fn camel_case(&mut self, _: &CamelCase, _: &mut Window, cx: &mut Context<Self>) {
        self.change_case(
            |t| {
                by_line(t, |words| {
                    words.iter().enumerate().map(|(i, w)| if i == 0 { lower(w) } else { capital(w) }).collect()
                })
            },
            cx,
        );
    }

    pub(super) fn pascal_case(&mut self, _: &PascalCase, _: &mut Window, cx: &mut Context<Self>) {
        self.change_case(|t| by_line(t, |w| join_words(w, "", capital)), cx);
    }

    pub(super) fn kebab_case(&mut self, _: &KebabCase, _: &mut Window, cx: &mut Context<Self>) {
        self.change_case(|t| by_line(t, |w| join_words(w, "-", lower)), cx);
    }

    pub(super) fn title_case(&mut self, _: &TitleCase, _: &mut Window, cx: &mut Context<Self>) {
        self.change_case(|t| by_line(t, |w| join_words(w, " ", capital)), cx);
    }

    /// The selection (or the word at the caret) in another case, still selected.
    fn change_case(&mut self, change: fn(&str) -> String, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            let range =
                if this.selection.is_empty() { this.word_at(this.selection.head) } else { this.selection.range() };
            if range.is_empty() {
                return;
            }
            let text = change(&this.buffer.slice(range.clone()));
            let len = text.chars().count();
            this.edit(range.clone(), &text, EditKind::Other, cx);
            this.selection = Selection { anchor: range.start, head: range.start + len };
        });
        self.touch(cx);
    }
}

/// Lines joined into one, and where the caret goes: the last place two lines met.
fn join(lines: &[String]) -> (String, usize) {
    let mut joined = lines[0].trim_end().to_string();
    let mut caret = joined.chars().count();
    for line in &lines[1..] {
        let line = line.trim();
        caret = joined.chars().count();
        if line.is_empty() {
            continue;
        }
        // No space inside parentheses and brackets, nor before a comma (braces keep theirs).
        let tight = joined.is_empty() || joined.ends_with(['(', '[']) || line.starts_with([')', ']', ',', ';', '.']);
        if !tight {
            joined.push(' ');
        }
        joined.push_str(line);
    }
    (joined, caret)
}

/// `line` with its indentation, counted in `from` levels, written in `to` ones; and how
/// many characters the indentation was, and is.
fn reindented(line: &str, from: crate::file_style::Indent, to: crate::file_style::Indent) -> (String, usize, usize) {
    let rest = line.trim_start_matches([' ', '\t']);
    let lead = &line[..line.len() - rest.len()];
    // A line of only spaces is left as it is.
    if rest.is_empty() || lead.is_empty() {
        return (line.to_string(), lead.len(), lead.len());
    }
    let mut columns = 0;
    for c in lead.chars() {
        columns = if c == '\t' { (columns / from.width() + 1) * from.width() } else { columns + 1 };
    }
    let new_lead = to.unit().repeat(columns / from.width()) + &" ".repeat(columns % from.width());
    (format!("{new_lead}{rest}"), lead.len(), new_lead.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn a_crlf_file_keeps_its_line_breaks_when_lines_are_sorted(cx: &mut gpui::TestAppContext) {
        use gpui::Focusable;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(crate::buffer::Buffer::from_text("b\r\na\r\nc\r\n"), Some("x.txt".into()), cx)
        });
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            e.selection = Selection { anchor: 0, head: 9 };
            e.sort_lines(&SortLines, window, cx);
            assert_eq!(e.buffer.to_string(), "a\r\nb\r\nc\r\n");
        });
    }

    #[gpui::test]
    fn lines_reverse_and_lose_their_repeats(cx: &mut gpui::TestAppContext) {
        use gpui::Focusable;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(crate::buffer::Buffer::from_text("b\na\nb\nc\na\nend\n"), Some("x.txt".into()), cx)
        });
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            e.selection = Selection { anchor: 0, head: 10 };
            e.remove_duplicate_lines(&RemoveDuplicateLines, window, cx);
            assert_eq!(e.buffer.to_string(), "b\na\nc\nend\n");
            assert_eq!(e.buffer.slice(e.selection.range()), "b\na\nc");
            e.reverse_lines(&ReverseLines, window, cx);
            assert_eq!(e.buffer.to_string(), "c\na\nb\nend\n");
        });
    }

    #[test]
    fn names_change_style() {
        assert_eq!(name_words("parseHTTPResponse"), ["parse", "HTTP", "Response"]);
        assert_eq!(name_words("user_id-v2 Name"), ["user", "id", "v2", "Name"]);
        let snake = |t: &str| by_line(t, |w| join_words(w, "_", lower));
        let pascal = |t: &str| by_line(t, |w| join_words(w, "", capital));
        assert_eq!(snake("parseHTTPResponse"), "parse_http_response");
        assert_eq!(pascal("user_id"), "UserId");
        assert_eq!(by_line("max-retry count", |w| join_words(w, " ", capital)), "Max Retry Count");
        assert_eq!(by_line("ÉtéChaud", |w| join_words(w, "-", lower)), "été-chaud");
        // Each line on its own, its indentation and its line break kept.
        assert_eq!(snake("aB\ncD"), "a_b\nc_d");
        assert_eq!(snake("    fooBar\r\n    bazQux"), "    foo_bar\r\n    baz_qux");
        assert_eq!(snake("a b\rc d"), "a_b\rc_d");
    }
    use crate::buffer::Buffer;
    use gpui::TestAppContext;
    use std::path::PathBuf;

    fn editor<'a>(cx: &'a mut TestAppContext, text: &str) -> (gpui::Entity<Editor>, &'a mut gpui::VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("x.rs")), cx))
    }

    fn selected(e: &Editor) -> String {
        e.buffer.slice(e.selection.range())
    }

    /// Trim Trailing Whitespace, the indentation made another, and ⌘/ on an empty line: each
    /// one undo, the caret kept in its text.
    #[gpui::test]
    fn tidying_lines(cx: &mut TestAppContext) {
        use crate::file_style::Indent;
        let (e, cx) = editor(cx, "fn a() {  \n  if x {\t\n      y(1,\n        2);\n  }\n   \n}\n");
        e.update_in(cx, |e, window, cx| {
            e.selection = Selection::caret(e.buffer.offset(1, 4));
            e.trim_trailing_whitespace(&TrimTrailingWhitespace, window, cx);
            assert_eq!(e.buffer.to_string(), "fn a() {\n  if x {\n      y(1,\n        2);\n  }\n\n}\n");
            assert_eq!(e.buffer.point(e.selection.head), (1, 4));
            // 2 spaces a level, to 4: the extra spaces lining `2` up stay extra.
            e.style.indent = Indent::Spaces(2);
            e.set_indent(Indent::Spaces(4), cx);
            assert_eq!(e.buffer.to_string(), "fn a() {\n    if x {\n            y(1,\n                2);\n    }\n\n}\n");
            assert_eq!(e.buffer.point(e.selection.head), (1, 6), "still after `if`");
            e.set_indent(Indent::Tabs, cx);
            assert_eq!(e.buffer.to_string(), "fn a() {\n\tif x {\n\t\t\ty(1,\n\t\t\t\t2);\n\t}\n\n}\n");
            // One undo each.
            e.step_history(true, cx);
            assert_eq!(e.buffer.to_string(), "fn a() {\n    if x {\n            y(1,\n                2);\n    }\n\n}\n");
            // ⌘/ on the empty line: a comment begins there.
            e.selection = Selection::caret(e.buffer.offset(5, 0));
            e.toggle_comment(cx);
            assert_eq!(e.buffer.line_text(5), "// ");
            assert_eq!(e.buffer.point(e.selection.head), (5, 3));
        });
    }

    #[gpui::test]
    fn selection_grows_by_syntax_and_shrinks_back(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "fn main() {\n    let x = foo(10, 2);\n}\n");
        e.update_in(cx, |e, window, cx| {
            e.selection = Selection::caret(e.buffer.offset(1, 17));
            let mut steps = Vec::new();
            for _ in 0..4 {
                e.expand_selection(&ExpandSelection, window, cx);
                steps.push(selected(e));
            }
            assert_eq!(steps, ["10", "(10, 2)", "foo(10, 2)", "let x = foo(10, 2);"]);
            e.shrink_selection(&ShrinkSelection, window, cx);
            e.shrink_selection(&ShrinkSelection, window, cx);
            assert_eq!(selected(e), "(10, 2)");
            // Moving the caret forgets the steps.
            e.selection = Selection::caret(0);
            e.shrink_selection(&ShrinkSelection, window, cx);
            assert_eq!(e.selection, Selection::caret(0));
        });
    }

    #[gpui::test]
    fn changes_are_visited_in_turn(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "a\n  b\nc\nd\ne\n");
        e.update_in(cx, |e, window, cx| {
            use crate::git::{Change, Hunk};
            e.git_hunks =
                vec![Hunk { change: Change::Modified, lines: 1..2 }, Hunk { change: Change::Added, lines: 3..5 }];
            e.next_change(&NextChange, window, cx);
            // On the changed line's code, past its indentation.
            assert_eq!(e.caret_point(), (1, 2));
            e.next_change(&NextChange, window, cx);
            assert_eq!(e.caret_point(), (3, 0));
            // Round to the first.
            e.next_change(&NextChange, window, cx);
            assert_eq!(e.caret_point(), (1, 2));
            e.previous_change(&PreviousChange, window, cx);
            assert_eq!(e.caret_point(), (3, 0));
        });
    }

    #[gpui::test]
    fn lines_open_join_sort_and_change_case(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, "fn a() {\n    b\n}\n");
        e.update_in(cx, |e, window, cx| {
            // ⌘↵ from the middle of a line that opens a block: the new line is indented.
            e.selection = Selection::caret(3);
            e.newline_below(&NewlineBelow, window, cx);
            assert_eq!(e.buffer.to_string(), "fn a() {\n    \n    b\n}\n");
            assert_eq!(e.caret_point(), (1, 4));
            e.newline_above(&NewlineAbove, window, cx);
            assert_eq!(e.buffer.to_string(), "fn a() {\n    \n    \n    b\n}\n");
            // ⌃J pulls the next line up.
            e.selection = Selection::caret(e.buffer.offset(0, 0));
            e.join_lines(&JoinLines, window, cx);
            e.join_lines(&JoinLines, window, cx);
            e.join_lines(&JoinLines, window, cx);
            assert_eq!(e.buffer.to_string(), "fn a() { b\n}\n");
            e.selection = Selection::caret(9);
            e.upper_case(&UpperCase, window, cx);
            assert_eq!(e.buffer.to_string(), "fn a() { B\n}\n");
            // ⌘⇧\ from after `{` to after `}`, then back.
            e.selection = Selection::caret(8);
            e.go_to_matching_bracket(&GoToMatchingBracket, window, cx);
            assert_eq!(e.selection.head, 12);
            e.buffer.replace(0..e.buffer.len_chars(), "pear\nApple\nfig\n");
            e.selection = Selection { anchor: 0, head: 15 };
            e.sort_lines(&SortLines, window, cx);
            assert_eq!(e.buffer.to_string(), "Apple\nfig\npear\n");
        });
    }

    #[test]
    fn grows_to_the_next_larger_range() {
        let ranges = || [3..5, 3..5, 2..9, 0..20].into_iter();
        assert_eq!(next_larger(&(4..4), ranges()), Some(3..5));
        // A range already selected grows past the ones the same size.
        assert_eq!(next_larger(&(3..5), ranges()), Some(2..9));
        assert_eq!(next_larger(&(0..20), ranges()), None);
    }

    #[test]
    fn joins_lines_with_one_space() {
        let lines = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(join(&lines(&["let x = foo(", "    a,", "    b", ");"])), ("let x = foo(a, b);".into(), 16));
        assert_eq!(join(&lines(&["one  ", "", "  two"])), ("one two".into(), 3));
    }
}
