//! Snippets from completions: `push(${1:value})$0` inserts `push(value)` with `value`
//! selected to type over. ⇥ moves to the next placeholder, ⇧⇥ back, and the last ⇥ (or
//! Esc) ends it there. A placeholder used twice is typed in both places at once.

use super::{Cursor, Editor, Selection};
use gpui::{App, Context, KeyBinding, Window, actions};
use std::collections::BTreeMap;
use std::ops::Range;

actions!(snippet, [NextPlaceholder, PreviousPlaceholder, EndSnippet]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Editor && in_snippet");
    cx.bind_keys([
        KeyBinding::new("tab", NextPlaceholder, ctx),
        KeyBinding::new("shift-tab", PreviousPlaceholder, ctx),
        KeyBinding::new("escape", EndSnippet, ctx),
    ]);
}

/// A snippet's text, and where its placeholders are in it (chars), in the order ⇥
/// visits them: $1, $2… then $0, which is always there (at the end if not written).
#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub text: String,
    pub stops: Vec<Vec<Range<usize>>>,
}

/// Reads LSP snippet syntax: `$1`, `${1}`, `${1:default}` (nested too), `${1|a,b|}`
/// (the first choice), and variables (`$TM_FILENAME`, `${NAME:default}`), which give
/// their default or nothing.
pub fn parse(snippet: &str) -> Parsed {
    parse_with(snippet, &|_| None)
}

/// The same, with `variable` giving the variables' values (see `snippets::variable`); one
/// it doesn't know gives its default or nothing.
pub fn parse_with(snippet: &str, variable: &dyn Fn(&str) -> Option<String>) -> Parsed {
    let mut parser = Parser {
        chars: snippet.chars().collect(),
        at: 0,
        text: String::new(),
        len: 0,
        stops: BTreeMap::new(),
        defaults: BTreeMap::new(),
        variable,
    };
    parser.until(false);
    let mut stops: Vec<Vec<Range<usize>>> =
        parser.stops.iter().filter(|(n, _)| **n > 0).map(|(_, r)| r.clone()).collect();
    stops.push(parser.stops.get(&0).cloned().unwrap_or_else(|| vec![parser.len..parser.len]));
    Parsed { text: parser.text, stops }
}

struct Parser<'a> {
    variable: &'a dyn Fn(&str) -> Option<String>,
    chars: Vec<char>,
    at: usize,
    text: String,
    /// `text` in chars.
    len: usize,
    stops: BTreeMap<u32, Vec<Range<usize>>>,
    /// Each placeholder's default text, for a `$1` repeating an earlier `${1:i}`.
    defaults: BTreeMap<u32, String>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn push(&mut self, c: char) {
        self.text.push(c);
        self.len += 1;
    }

    fn number(&mut self) -> u32 {
        let mut n = 0u32;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            n = n.saturating_mul(10).saturating_add(d);
            self.at += 1;
        }
        n
    }

    fn name(&mut self) -> String {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            self.at += 1;
        }
        self.chars[start..self.at].iter().collect()
    }

    /// A variable's value, as text (nothing in it is snippet syntax).
    fn value(&mut self, value: &str) {
        value.chars().for_each(|c| self.push(c));
    }

    fn stop(&mut self, n: u32, range: Range<usize>) {
        self.stops.entry(n).or_default().push(range);
    }

    /// `$1` or `${1}`: empty, or the same text as an earlier `${1:…}`.
    fn mirror(&mut self, n: u32) {
        let start = self.len;
        if let Some(text) = self.defaults.get(&n).cloned() {
            text.chars().for_each(|c| self.push(c));
        }
        self.stop(n, start..self.len);
    }

    /// Reads up to the end, or (`nested`) up to the `}` closing a placeholder.
    fn until(&mut self, nested: bool) {
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '\\' => match self.peek() {
                    Some(next @ ('$' | '}' | '\\' | ',' | '|')) => {
                        self.at += 1;
                        self.push(next);
                    }
                    _ => self.push('\\'),
                },
                '}' if nested => return,
                '$' => self.dollar(),
                c => self.push(c),
            }
        }
    }

    fn dollar(&mut self) {
        match self.peek() {
            Some(c) if c.is_ascii_digit() => {
                let n = self.number();
                self.mirror(n);
            }
            Some('{') => {
                self.at += 1;
                if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    let n = self.number();
                    let (start, start_byte) = (self.len, self.text.len());
                    match self.peek() {
                        Some(':') => {
                            self.at += 1;
                            self.until(true);
                        }
                        Some('|') => {
                            self.at += 1;
                            self.choice();
                        }
                        _ => {
                            self.skip_past('}');
                            return self.mirror(n);
                        }
                    }
                    self.defaults.insert(n, self.text[start_byte..].to_string());
                    self.stop(n, start..self.len);
                } else {
                    // A variable: its value, else its default if it has one.
                    let name = self.name();
                    let value = (self.variable)(&name);
                    match (value, self.peek() == Some(':')) {
                        (Some(value), default) => {
                            self.value(&value);
                            if default {
                                self.skip_nested();
                            } else {
                                self.skip_past('}');
                            }
                        }
                        (None, true) => {
                            self.at += 1;
                            self.until(true);
                        }
                        (None, false) => self.skip_past('}'),
                    }
                }
            }
            Some(c) if c.is_alphabetic() || c == '_' => {
                let name = self.name();
                if let Some(value) = (self.variable)(&name) {
                    self.value(&value);
                }
            }
            _ => self.push('$'),
        }
    }

    /// `a,b|}`: the first choice goes in.
    fn choice(&mut self) {
        let mut first = true;
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '|' if self.peek() == Some('}') => {
                    self.at += 1;
                    return;
                }
                ',' => first = false,
                '\\' => {
                    if let Some(next) = self.peek() {
                        self.at += 1;
                        if first {
                            self.push(next);
                        }
                    }
                }
                c if first => self.push(c),
                _ => {}
            }
        }
    }

    /// Past the `}` closing this `${…}`, over any nested in it.
    fn skip_nested(&mut self) {
        let mut depth = 1;
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '\\' => self.at += 1,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn skip_past(&mut self, close: char) {
        while let Some(c) = self.peek() {
            self.at += 1;
            if c == close {
                return;
            }
        }
    }
}

/// A snippet being filled in: its placeholders (byte ranges), kept up to date with edits.
pub(super) struct Session {
    stops: Vec<Vec<Range<usize>>>,
    current: usize,
    revision: u64,
}

/// Where a placeholder goes after `edit`: typing inside it, or right at either edge,
/// grows it; replacing it shrinks it to what was typed; edits elsewhere move it.
fn map_stop(range: Range<usize>, edit: &crate::buffer::Edit) -> Range<usize> {
    let (start, old_end, new_end) = (edit.start_byte, edit.old_end_byte, edit.new_end_byte);
    let shift = |p: usize| p - old_end + new_end;
    let inserting = start == old_end;
    let first = if range.start < start || (inserting && range.start == start) {
        range.start
    } else if range.start >= old_end {
        shift(range.start)
    } else {
        start
    };
    let last = if range.end < start || (!inserting && range.end == start) {
        range.end
    } else if range.end > old_end {
        shift(range.end)
    } else {
        new_end
    };
    first..last.max(first)
}

impl Editor {
    /// Inserts a parsed snippet at `at` (chars) in place of `replace`, and selects its first
    /// placeholder. Called inside an undo step the caller opened.
    pub(super) fn start_snippet(&mut self, at: usize, parsed: Parsed, cx: &mut Context<Self>) {
        let rope = self.buffer.rope();
        let byte = |c: usize| rope.char_to_byte(c.min(rope.len_chars()));
        let stops: Vec<Vec<Range<usize>>> = parsed
            .stops
            .iter()
            .map(|ranges| ranges.iter().map(|r| byte(at + r.start)..byte(at + r.end)).collect())
            .collect();
        // Only $0: nothing to fill in, the caret just goes there.
        if stops.len() == 1 {
            let end = rope.byte_to_char(stops[0][0].start);
            self.selection = Selection::caret(end);
            return;
        }
        self.snippet = Some(Session { stops, current: 0, revision: self.buffer.revision() });
        self.select_placeholder(cx);
    }

    #[cfg(test)]
    pub fn in_snippet(&self) -> bool {
        self.snippet.is_some()
    }

    /// Brings the placeholders up to date; ends the snippet if an edit broke one.
    fn sync_snippet(&mut self) -> bool {
        let Some(session) = &mut self.snippet else { return false };
        let revision = self.buffer.revision();
        if session.revision != revision {
            let Some(edits) = self.buffer.edits_since(session.revision) else {
                self.snippet = None;
                return false;
            };
            let edits: Vec<_> = edits.collect();
            for ranges in &mut session.stops {
                for range in ranges.iter_mut() {
                    *range = edits.iter().fold(range.clone(), |r, e| map_stop(r, e));
                }
            }
            session.revision = revision;
        }
        true
    }

    /// The current placeholder selected, with a cursor in each place it appears.
    fn select_placeholder(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.snippet else { return };
        let rope = self.buffer.rope();
        let chars = |r: &Range<usize>| {
            let (a, b) = (rope.byte_to_char(r.start), rope.byte_to_char(r.end));
            Selection { anchor: a, head: b }
        };
        let ranges = &session.stops[session.current];
        self.selection = chars(&ranges[0]);
        self.extra = ranges[1..].iter().map(|r| Cursor { selection: chars(r), goal: None }).collect();
        self.goal_column = None;
        // The last stop ($0) is where the snippet ends.
        if session.current + 1 == session.stops.len() {
            self.snippet = None;
        }
        self.touch(cx);
    }

    /// Whether the caret is still in the snippet's current placeholder.
    fn caret_in_placeholder(&self) -> bool {
        let Some(session) = &self.snippet else { return false };
        let rope = self.buffer.rope();
        let head = rope.char_to_byte(self.selection.head);
        session.stops[session.current].iter().any(|r| r.start <= head && head <= r.end)
    }

    fn move_placeholder(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.completion.is_some() && step > 0 {
            return window.dispatch_action(Box::new(super::ConfirmCompletion), cx);
        }
        // Moved away from the snippet: ⇥ is just ⇥ again.
        if !self.sync_snippet() || !self.caret_in_placeholder() {
            self.snippet = None;
            cx.notify();
            let action: Box<dyn gpui::Action> = if step > 0 { Box::new(super::Tab) } else { Box::new(super::Outdent) };
            return window.dispatch_action(action, cx);
        }
        let Some(session) = &mut self.snippet else { return };
        session.current = (session.current as isize + step).clamp(0, session.stops.len() as isize - 1) as usize;
        self.select_placeholder(cx);
    }

    pub(super) fn next_placeholder(&mut self, _: &NextPlaceholder, window: &mut Window, cx: &mut Context<Self>) {
        self.move_placeholder(1, window, cx);
    }

    pub(super) fn previous_placeholder(
        &mut self,
        _: &PreviousPlaceholder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_placeholder(-1, window, cx);
    }

    pub(super) fn end_snippet(&mut self, _: &EndSnippet, window: &mut Window, cx: &mut Context<Self>) {
        if self.completion.is_some() {
            return window.dispatch_action(Box::new(super::CancelCompletion), cx);
        }
        self.snippet = None;
        self.extra.clear();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use gpui::{Focusable, TestAppContext};
    use std::path::PathBuf;

    #[gpui::test]
    fn tab_walks_the_placeholders_then_is_tab_again(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text("v.\n"), Some(PathBuf::from("x.txt")), cx));
        let selected = |e: &Editor| e.buffer.slice(e.selection.range());
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            // As accepting `insert(…)` does: the text in, then its placeholders.
            let parsed = parse("insert(${1:key}, ${2:value})$0");
            e.buffer.replace(2..2, &parsed.text);
            e.start_snippet(2, parsed, cx);
            assert_eq!(selected(e), "key");
        });
        cx.run_until_parked();
        cx.simulate_input("name");
        cx.simulate_keystrokes("tab");
        e.update(cx, |e, _| assert_eq!(selected(e), "value"));
        cx.simulate_input("42");
        // Back to the first, still holding what was typed.
        cx.simulate_keystrokes("shift-tab");
        e.update(cx, |e, _| assert_eq!(selected(e), "name"));
        cx.simulate_keystrokes("tab tab");
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "v.insert(name, 42)\n");
            assert_eq!(e.selection.head, 18);
            assert!(!e.in_snippet());
        });
        // Done: ⇥ indents again.
        cx.simulate_keystrokes("tab");
        e.update(cx, |e, _| assert_ne!(e.buffer.to_string(), "v.insert(name, 42)\n"));
    }

    #[test]
    fn reads_placeholders_choices_and_variables() {
        let p = parse("push(${1:value})$0");
        assert_eq!(p.text, "push(value)");
        assert_eq!(p.stops, vec![vec![5..10], vec![11..11]]);
        // Nested, repeated, and the end added when not written.
        let p = parse("for ${1:i} in ${2:0..${3:n}} { $1 }");
        assert_eq!(p.text, "for i in 0..n { i }");
        assert_eq!(p.stops, vec![vec![4..5, 16..17], vec![9..13], vec![12..13], vec![19..19]]);
        let p = parse("${1|one,two|} ${TM_FILENAME} ${NAME:anon} \\$x");
        assert_eq!(p.text, "one  anon $x");
        assert_eq!(p.stops[0], vec![0..3]);
    }

    fn edit(start: usize, old_end: usize, new_end: usize) -> crate::buffer::Edit {
        crate::buffer::Edit {
            start_byte: start,
            old_end_byte: old_end,
            new_end_byte: new_end,
            start: (0, 0),
            old_end: (0, 0),
            new_end: (0, 0),
            lsp_range: None,
            text: String::new(),
        }
    }

    #[test]
    fn placeholders_grow_with_typing_and_move_with_edits() {
        // Typing over a selected placeholder, then more at its end, then at its start.
        assert_eq!(map_stop(5..10, &edit(5, 10, 6)), 5..6);
        assert_eq!(map_stop(5..6, &edit(6, 6, 7)), 5..7);
        assert_eq!(map_stop(5..7, &edit(5, 5, 6)), 5..8);
        // Into an empty one.
        assert_eq!(map_stop(5..5, &edit(5, 5, 8)), 5..8);
        // Before it (ending right at it), after it (starting right at it), across its edge.
        assert_eq!(map_stop(5..10, &edit(0, 2, 0)), 3..8);
        assert_eq!(map_stop(5..10, &edit(3, 5, 4)), 4..9);
        assert_eq!(map_stop(5..10, &edit(10, 12, 10)), 5..10);
        assert_eq!(map_stop(5..10, &edit(8, 12, 8)), 5..8);
        // Swallowed by an edit around it (typing over the placeholder holding it).
        assert_eq!(map_stop(7..8, &edit(5, 10, 6)), 5..6);
    }
}
