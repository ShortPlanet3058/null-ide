//! ⌃⌥↑ and ⌃⌥↓: the number at the caret (or the next one on its line) goes up or down
//! by one, by ten with ⇧; on a decimal's fraction, by its last place (`0.5` → `0.6`). Its
//! zeros, decimals and hex case stay as written. On `true`, `yes` or `on`, the value flips.
//! With a selection, every number in it steps.

use super::{EditKind, Editor, Selection};
use gpui::{App, Context, KeyBinding, Window, actions};
use std::ops::Range;

actions!(numbers, [Increment, Decrement, IncrementByTen, DecrementByTen]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Editor");
    cx.bind_keys([
        KeyBinding::new("ctrl-alt-up", Increment, ctx),
        KeyBinding::new("ctrl-alt-down", Decrement, ctx),
        KeyBinding::new("ctrl-alt-shift-up", IncrementByTen, ctx),
        KeyBinding::new("ctrl-alt-shift-down", DecrementByTen, ctx),
    ]);
}

/// Values that flip into each other.
const FLIPS: &[(&str, &str)] = &[
    ("true", "false"),
    ("True", "False"),
    ("TRUE", "FALSE"),
    ("yes", "no"),
    ("Yes", "No"),
    ("YES", "NO"),
    ("on", "off"),
    ("On", "Off"),
    ("ON", "OFF"),
];

fn flipped(word: &str) -> Option<&'static str> {
    FLIPS.iter().find_map(|&(a, b)| {
        if word == a {
            Some(b)
        } else if word == b {
            Some(a)
        } else {
            None
        }
    })
}

/// A number written in a line: where (bytes, its sign included), and where its fraction
/// starts, if it has one.
#[derive(Debug, PartialEq)]
struct Number {
    range: Range<usize>,
    hex: bool,
    point: Option<usize>,
}

/// The numbers in `text`, in order. A version (`1.2.3`) is numbers apart, not decimals; a
/// `-` is a sign only where it can't be a minus (`(-1`, `= -1`, not `a-1`).
fn numbers(text: &str) -> Vec<Number> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    // In a version, each part is a whole number.
    let mut version = false;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            if b[i] != b'.' {
                version = false;
            }
            i += 1;
            continue;
        }
        let start = i;
        let word_before = start > 0 && (b[start - 1].is_ascii_alphanumeric() || b[start - 1] == b'_');
        if b[i] == b'0' && matches!(b.get(i + 1), Some(b'x' | b'X')) && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            let mut end = i + 2;
            while end < b.len() && b[end].is_ascii_hexdigit() {
                end += 1;
            }
            out.push(Number { range: start..end, hex: true, point: None });
            i = end;
            continue;
        }
        let digits_end = |mut j: usize| {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            j
        };
        let mut end = digits_end(i);
        let mut point = None;
        let fraction = b.get(end) == Some(&b'.') && b.get(end + 1).is_some_and(u8::is_ascii_digit);
        if fraction && !version {
            let after = digits_end(end + 1);
            let more = b.get(after) == Some(&b'.') && b.get(after + 1).is_some_and(u8::is_ascii_digit);
            if more || (start > 0 && b[start - 1] == b'.') {
                version = true;
            } else {
                point = Some(end);
                end = after;
            }
        }
        let signed = !word_before
            && start > 0
            && b[start - 1] == b'-'
            && !(start > 1 && (b[start - 2].is_ascii_alphanumeric() || matches!(b[start - 2], b'_' | b')' | b']')));
        out.push(Number { range: if signed { start - 1..end } else { start..end }, hex: false, point });
        i = end;
    }
    out
}

/// The number written `text` stepped by `delta` (in its last place when `in_fraction`),
/// written the same way. None when it can't be (a hex number below zero, too many digits).
fn stepped(text: &str, number: &Number, delta: i64, in_fraction: bool) -> Option<String> {
    let base = number.range.start;
    if number.hex {
        let digits = &text[2..];
        let value = i128::from_str_radix(digits, 16).ok()? + i128::from(delta);
        if value < 0 || digits.len() > 24 {
            return None;
        }
        let upper = digits.chars().any(|c| c.is_ascii_uppercase());
        let written = if upper { format!("{value:X}") } else { format!("{value:x}") };
        return Some(format!("{}{written:0>width$}", &text[..2], width = digits.len()));
    }
    let negative = text.starts_with('-');
    let unsigned = text.trim_start_matches('-');
    let (whole, places) = match number.point {
        Some(p) => (&text[text.len() - unsigned.len()..p - base], &text[p - base + 1..]),
        None => (unsigned, ""),
    };
    if whole.len() + places.len() > 30 {
        return None;
    }
    let scaled: i128 = format!("{whole}{places}").parse().ok()?;
    let unit = if in_fraction { 1 } else { 10i128.pow(places.len() as u32) };
    let value = if negative { -scaled } else { scaled } + i128::from(delta) * unit;
    let digits = format!("{:0>width$}", value.unsigned_abs(), width = places.len() + 1);
    let (new_whole, new_places) = digits.split_at(digits.len() - places.len());
    // `007` stays three digits wide.
    let padded = if whole.len() > 1 && whole.starts_with('0') {
        format!("{new_whole:0>width$}", width = whole.len())
    } else {
        new_whole.to_string()
    };
    let sign = if value < 0 { "-" } else { "" };
    Some(if places.is_empty() { format!("{sign}{padded}") } else { format!("{sign}{padded}.{new_places}") })
}

/// For the caret at byte `caret` in `line`: what to replace (bytes) and with what, and
/// whether it was a number found further on the line.
fn step_at(line: &str, caret: usize, delta: i64) -> Option<(Range<usize>, String, bool)> {
    let all = numbers(line);
    let at = all.iter().find(|n| n.range.start <= caret && caret < n.range.end).or_else(|| {
        // Right after one (`42|`), or else the next one along.
        all.iter().find(|n| n.range.end == caret).or_else(|| all.iter().find(|n| n.range.start > caret))
    })?;
    let text = &line[at.range.clone()];
    let in_fraction = at.point.is_some_and(|p| caret > p);
    let ahead = at.range.start > caret;
    stepped(text, at, delta, in_fraction && !ahead).map(|new| (at.range.clone(), new, ahead))
}

/// Every number in `text` stepped in its units.
fn step_all(text: &str, delta: i64) -> Option<String> {
    let all = numbers(text);
    if all.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for n in &all {
        out.push_str(&text[last..n.range.start]);
        let written = &text[n.range.clone()];
        out.push_str(&stepped(written, n, delta, false).unwrap_or_else(|| written.to_string()));
        last = n.range.end;
    }
    out.push_str(&text[last..]);
    Some(out)
}

impl Editor {
    pub(super) fn increment(&mut self, _: &Increment, _: &mut Window, cx: &mut Context<Self>) {
        self.step_numbers(1, cx);
    }

    pub(super) fn decrement(&mut self, _: &Decrement, _: &mut Window, cx: &mut Context<Self>) {
        self.step_numbers(-1, cx);
    }

    pub(super) fn increment_by_ten(&mut self, _: &IncrementByTen, _: &mut Window, cx: &mut Context<Self>) {
        self.step_numbers(10, cx);
    }

    pub(super) fn decrement_by_ten(&mut self, _: &DecrementByTen, _: &mut Window, cx: &mut Context<Self>) {
        self.step_numbers(-10, cx);
    }

    fn step_numbers(&mut self, delta: i64, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            if !this.selection.is_empty() {
                let mut range = this.selection.range();
                // A number selected without its sign (`-5`, double-clicked): the sign goes with it.
                let at = |i: usize| this.buffer.char_at(i);
                let glued = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | ')' | ']'));
                if range.start > 0
                    && at(range.start - 1) == Some('-')
                    && at(range.start).is_some_and(|c| c.is_ascii_digit())
                    && !(range.start > 1 && glued(at(range.start - 2)))
                {
                    range.start -= 1;
                }
                let Some(text) = step_all(&this.buffer.slice(range.clone()), delta) else { return };
                let len = text.chars().count();
                this.edit(range.clone(), &text, EditKind::Other, cx);
                this.selection = Selection { anchor: range.start, head: range.start + len };
                return;
            }
            let caret = this.selection.head;
            let (line, col) = this.buffer.point(caret);
            let start = this.buffer.line_to_char(line);
            let text = this.buffer.line_text(line);
            let caret_byte = text.char_indices().nth(col).map_or(text.len(), |(i, _)| i);
            let chars = |bytes: Range<usize>| {
                start + text[..bytes.start].chars().count()..start + text[..bytes.end].chars().count()
            };
            // A value to flip right at the caret comes before a number further on the line.
            let word = this.word_at(caret);
            let flip = flipped(&this.buffer.slice(word.clone()));
            if let Some((bytes, new, ahead)) = step_at(&text, caret_byte, delta).filter(|s| !(s.2 && flip.is_some())) {
                let range = chars(bytes);
                let range_start = range.start;
                // The caret stays on the same digit, counted from the right; on a number found
                // further on, it goes to its end.
                let from_end = if ahead { 0 } else { range.end - caret };
                let end = range.start + new.chars().count();
                this.edit(range, &new, EditKind::Other, cx);
                // On it still, however much shorter it got (`10` → `9`).
                this.selection = Selection::caret(end.saturating_sub(from_end).max(range_start));
                return;
            }
            if let Some(flip) = flip {
                this.edit(word.clone(), flip, EditKind::Other, cx);
                this.selection = Selection::caret((word.start + flip.chars().count()).min(caret.max(word.start)));
            }
        });
        self.touch(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(line: &str, caret: usize, delta: i64) -> Option<String> {
        step_at(line, caret, delta).map(|(r, new, _)| format!("{}{new}{}", &line[..r.start], &line[r.end..]))
    }

    #[test]
    fn numbers_step_as_written() {
        assert_eq!(step("width: 10px;", 8, 1).as_deref(), Some("width: 11px;"));
        assert_eq!(step("width: 10px;", 0, -1).as_deref(), Some("width: 9px;"), "the next one on the line");
        assert_eq!(step("x = 0", 5, -1).as_deref(), Some("x = -1"));
        assert_eq!(step("x = -1", 5, 1).as_deref(), Some("x = 0"));
        assert_eq!(step("x = (-5)", 6, 10).as_deref(), Some("x = (5)"));
        assert_eq!(step("a-1", 2, 1).as_deref(), Some("a-2"), "a minus, not a sign");
        assert_eq!(step("n - 1", 4, -2).as_deref(), Some("n - -1"));
        assert_eq!(step("id007", 3, 1).as_deref(), Some("id008"));
        assert_eq!(step("v = 099", 5, 1).as_deref(), Some("v = 100"));
        assert_eq!(step("<h1>", 2, 1).as_deref(), Some("<h2>"));
        assert_eq!(step("opacity: 0.5", 9, 1).as_deref(), Some("opacity: 1.5"), "on the whole part");
        assert_eq!(step("opacity: 0.5", 11, 1).as_deref(), Some("opacity: 0.6"), "on the fraction");
        assert_eq!(step("x 0.95", 6, 1).as_deref(), Some("x 0.96"));
        assert_eq!(step("x 0.10", 5, -1).as_deref(), Some("x 0.09"));
        assert_eq!(step("x 0.1", 5, -2).as_deref(), Some("x -0.1"));
        assert_eq!(step("x 0.0", 2, -1).as_deref(), Some("x -1.0"));
        assert_eq!(step("version 1.2.3", 12, 1).as_deref(), Some("version 1.2.4"));
        assert_eq!(step("version 1.2.3", 10, 1).as_deref(), Some("version 1.3.3"));
        assert_eq!(step("color 0xff", 8, 1).as_deref(), Some("color 0x100"));
        assert_eq!(step("0x0F", 2, 1).as_deref(), Some("0x10"));
        assert_eq!(step("0x0A", 2, 1).as_deref(), Some("0x0B"));
        assert_eq!(step("0x0", 2, -1), None, "no hex below zero");
        assert_eq!(step("no numbers", 3, 1), None);
        assert_eq!(step("é 9", 2, 1).as_deref(), Some("é 10"));
    }

    #[test]
    fn every_number_in_a_selection_steps() {
        assert_eq!(step_all("margin: 1px 2px -3px", 1).as_deref(), Some("margin: 2px 3px -2px"));
        assert_eq!(step_all("1.2.3", 1).as_deref(), Some("2.3.4"));
        assert_eq!(step_all("none", 1), None);
    }

    #[test]
    fn values_flip() {
        assert_eq!(flipped("true"), Some("false"));
        assert_eq!(flipped("False"), Some("True"));
        assert_eq!(flipped("off"), Some("on"));
        assert_eq!(flipped("maybe"), None);
    }

    #[gpui::test]
    fn the_keys_step_and_flip(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let text = "let n = 9;\nlet on = true;\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(crate::buffer::Buffer::from_text(text), Some("a.rs".into()), cx));
        e.update_in(cx, |e, window, cx| {
            window.focus(&gpui::Focusable::focus_handle(e, cx));
            e.selection = Selection::caret(0);
        });
        cx.simulate_keystrokes("ctrl-alt-up");
        assert_eq!(e.read_with(cx, |e, _| (e.buffer.line_text(0), e.selection.head)), ("let n = 10;".into(), 10));
        cx.simulate_keystrokes("ctrl-alt-shift-down ctrl-alt-shift-down");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.line_text(0)), "let n = -10;");
        e.update(cx, |e, _| e.selection = Selection::caret(e.buffer.to_string().find("true").unwrap() + 1));
        // `true` under the caret flips, though a number follows on the line.
        e.update(cx, |e, cx| {
            let at = e.buffer.line_to_char(1);
            e.buffer.replace(at..at + e.buffer.line_len(1), "let on = true; // level 2");
            e.text_changed(cx);
        });
        cx.simulate_keystrokes("ctrl-alt-down");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.line_text(1)), "let on = false; // level 2");
        // `10` down to `9`: the caret stays on it.
        e.update(cx, |e, _| e.selection = Selection::caret(8));
        cx.simulate_keystrokes("ctrl-alt-up ctrl-alt-up");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.line_text(0)), "let n = -8;");
        e.update(cx, |e, _| e.selection = Selection::caret(9));
        cx.simulate_keystrokes("ctrl-alt-shift-up");
        assert_eq!(e.read_with(cx, |e, _| (e.buffer.line_text(0), e.selection.head)), ("let n = 2;".into(), 8));
        // `5` selected without its sign: `-5` steps.
        cx.simulate_keystrokes("ctrl-alt-shift-down ctrl-alt-shift-down");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.line_text(0)), "let n = -18;");
        e.update(cx, |e, _| e.selection = Selection { anchor: 9, head: 11 });
        cx.simulate_keystrokes("ctrl-alt-up");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.line_text(0)), "let n = -17;");
    }
}
