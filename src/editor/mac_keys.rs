//! The ⌃ keys every macOS text field knows: ⌃A and ⌃E to the line's ends, ⌃F ⌃B ⌃N ⌃P to
//! move, ⌃D and ⌃H to delete, ⌃K to cut the rest of the line and ⌃Y to put it back, ⌃T to
//! swap two letters, ⌃O to open a line and ⌃L to center the caret's line; ⌘J brings the
//! selection back into view (Jump to Selection), as in the Mac's own apps. ⌃⌥← → go by the
//! parts of a name (`parse` `Http` `Request`, `snake` `case`), ⌃⌥⇧ selecting, ⌃⌥⌫ deleting.

use super::{
    Backspace, CompletionNext, CompletionPrevious, Delete, EditKind, Editor, MoveDown, MoveLeft, MoveLineEnd,
    MoveLineStart, MoveRight, MoveUp, PageDown, SelectDown, SelectLeft, SelectLineEnd, SelectLineStart, SelectRight,
    SelectUp, Selection,
};
use gpui::{App, Context, Global, KeyBinding, Window, actions};

actions!(
    editor,
    [
        DeleteToLineEnd,
        Yank,
        Transpose,
        OpenLine,
        CenterCaretLine,
        JumpToSelection,
        MoveSubwordLeft,
        MoveSubwordRight,
        SelectSubwordLeft,
        SelectSubwordRight,
        DeleteSubwordLeft
    ]
);

pub fn bind_keys(cx: &mut App) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let ctx = Some("Editor");
    cx.bind_keys([
        KeyBinding::new("ctrl-l", CenterCaretLine, ctx),
        KeyBinding::new("ctrl-alt-left", MoveSubwordLeft, ctx),
        KeyBinding::new("ctrl-alt-right", MoveSubwordRight, ctx),
        KeyBinding::new("ctrl-alt-shift-left", SelectSubwordLeft, ctx),
        KeyBinding::new("ctrl-alt-shift-right", SelectSubwordRight, ctx),
        KeyBinding::new("ctrl-alt-backspace", DeleteSubwordLeft, ctx),
        KeyBinding::new("cmd-j", JumpToSelection, ctx),
        KeyBinding::new("ctrl-a", MoveLineStart, ctx),
        KeyBinding::new("ctrl-e", MoveLineEnd, ctx),
        KeyBinding::new("ctrl-shift-a", SelectLineStart, ctx),
        KeyBinding::new("ctrl-shift-e", SelectLineEnd, ctx),
        KeyBinding::new("ctrl-f", MoveRight, ctx),
        KeyBinding::new("ctrl-b", MoveLeft, ctx),
        KeyBinding::new("ctrl-n", MoveDown, ctx),
        KeyBinding::new("ctrl-p", MoveUp, ctx),
        KeyBinding::new("ctrl-shift-f", SelectRight, ctx),
        KeyBinding::new("ctrl-shift-b", SelectLeft, ctx),
        KeyBinding::new("ctrl-shift-n", SelectDown, ctx),
        KeyBinding::new("ctrl-shift-p", SelectUp, ctx),
        KeyBinding::new("ctrl-v", PageDown, ctx),
        KeyBinding::new("ctrl-d", Delete, ctx),
        KeyBinding::new("ctrl-h", Backspace, ctx),
        KeyBinding::new("ctrl-k", DeleteToLineEnd, ctx),
        KeyBinding::new("ctrl-y", Yank, ctx),
        KeyBinding::new("ctrl-t", Transpose, ctx),
        KeyBinding::new("ctrl-o", OpenLine, ctx),
        // After the keys above, so the suggestion list keeps its own.
        KeyBinding::new("ctrl-n", CompletionNext, Some("Editor && showing_completions")),
        KeyBinding::new("ctrl-p", CompletionPrevious, Some("Editor && showing_completions")),
    ]);
}

/// What ⌃K last cut, for ⌃Y: apart from the clipboard, as on macOS.
#[derive(Default)]
struct Killed(String);

impl Global for Killed {}

/// Where the next part of a name ends, going right from `at` in `text`: past spaces, then
/// one run of marks, or one part of a word (`parse`, `Http`, an acronym like `HTML` before
/// `Parser`, a run of digits), underscores before it taken along.
fn subword_right(text: &[char], at: usize) -> usize {
    let mut i = at;
    while i < text.len() && text[i].is_whitespace() {
        i += 1;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    if i < text.len() && !word(text[i]) {
        while i < text.len() && !word(text[i]) && !text[i].is_whitespace() {
            i += 1;
        }
        return i;
    }
    while i < text.len() && text[i] == '_' {
        i += 1;
    }
    let start = i;
    if i < text.len() && text[i].is_uppercase() {
        while i < text.len() && text[i].is_uppercase() {
            i += 1;
        }
        // An acronym before a capitalised part (`HTMLParser`): its last capital is the part's.
        if i - start > 1 && i < text.len() && text[i].is_lowercase() {
            return i - 1;
        }
    }
    if i < text.len() && text[i].is_ascii_digit() {
        while i < text.len() && text[i].is_ascii_digit() {
            i += 1;
        }
        return i;
    }
    while i < text.len() && text[i].is_alphabetic() && !text[i].is_uppercase() {
        i += 1;
    }
    i
}

/// Where the part of a name before `at` starts (see `subword_right`).
fn subword_left(text: &[char], at: usize) -> usize {
    let mut i = at;
    while i > 0 && text[i - 1].is_whitespace() {
        i -= 1;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    if i > 0 && !word(text[i - 1]) {
        while i > 0 && !word(text[i - 1]) && !text[i - 1].is_whitespace() {
            i -= 1;
        }
        return i;
    }
    while i > 0 && text[i - 1] == '_' {
        i -= 1;
    }
    if i > 0 && text[i - 1].is_ascii_digit() {
        while i > 0 && text[i - 1].is_ascii_digit() {
            i -= 1;
        }
        return i;
    }
    if i > 0 && text[i - 1].is_uppercase() {
        // An acronym: all its capitals.
        while i > 0 && text[i - 1].is_uppercase() {
            i -= 1;
        }
        return i;
    }
    while i > 0 && text[i - 1].is_alphabetic() && !text[i - 1].is_uppercase() {
        i -= 1;
    }
    // The part's own capital (`Request`).
    if i > 0 && text[i - 1].is_uppercase() {
        i -= 1;
    }
    i
}

impl Editor {
    /// Where ⌃⌥→ goes from `offset`: the end of the next part of a name, on the line (at its
    /// end, the next line's start).
    fn subword_right_of(&self, offset: usize) -> usize {
        let (line, col) = self.buffer.point(offset);
        let text: Vec<char> = self.buffer.line_text(line).chars().collect();
        if col >= text.len() {
            return (offset + 1).min(self.buffer.len_chars()).max(offset);
        }
        self.buffer.line_to_char(line) + subword_right(&text, col)
    }

    fn subword_left_of(&self, offset: usize) -> usize {
        let (line, col) = self.buffer.point(offset);
        if col == 0 {
            return offset.saturating_sub(1);
        }
        let text: Vec<char> = self.buffer.line_text(line).chars().collect();
        self.buffer.line_to_char(line) + subword_left(&text, col)
    }

    pub(super) fn move_subword_left(&mut self, _: &MoveSubwordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| this.move_head(this.subword_left_of(this.selection.head), false, cx));
    }

    pub(super) fn move_subword_right(&mut self, _: &MoveSubwordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| this.move_head(this.subword_right_of(this.selection.head), false, cx));
    }

    pub(super) fn select_subword_left(&mut self, _: &SelectSubwordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| this.move_head(this.subword_left_of(this.selection.head), true, cx));
    }

    pub(super) fn select_subword_right(&mut self, _: &SelectSubwordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| this.move_head(this.subword_right_of(this.selection.head), true, cx));
    }

    pub(super) fn delete_subword_left(&mut self, _: &DeleteSubwordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            this.delete_or(|this| this.subword_left_of(this.selection.head)..this.selection.head, cx);
        });
    }

    /// ⌃L: the caret's line in the middle of the window.
    pub(super) fn center_caret_line(&mut self, _: &CenterCaretLine, _: &mut Window, cx: &mut Context<Self>) {
        self.center_once = true;
        self.autoscroll = true;
        cx.notify();
    }

    /// ⌘J: the selection back in view after scrolling away from it.
    pub(super) fn jump_to_selection(&mut self, _: &JumpToSelection, _: &mut Window, cx: &mut Context<Self>) {
        self.autoscroll = true;
        self.reveal_only = false;
        cx.notify();
    }

    /// The rest of the line, or the line break at its end; ⌃K again adds to what ⌃Y puts back.
    pub(super) fn delete_to_line_end(&mut self, _: &DeleteToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let again = self.extra.is_empty() && self.killed_at == Some((self.buffer.revision(), self.selection.head));
        let mut cut = Vec::new();
        self.for_each_cursor(cx, |this, cx| {
            let range = if this.selection.is_empty() {
                let caret = this.selection.head;
                let (line, _) = this.buffer.point(caret);
                let end = this.buffer.line_to_char(line) + this.buffer.line_len(line);
                if caret < end { caret..end } else { caret..this.buffer.line_to_char(line + 1) }
            } else {
                this.selection.range()
            };
            if range.is_empty() {
                return;
            }
            cut.push(this.buffer.slice(range.clone()));
            this.edit(range, "", EditKind::Other, cx);
        });
        if cut.is_empty() {
            return;
        }
        // The cursors went last first.
        cut.reverse();
        let killed = &mut cx.default_global::<Killed>().0;
        if !again {
            killed.clear();
        }
        killed.push_str(&cut.join("\n"));
        self.killed_at = self.extra.is_empty().then(|| (self.buffer.revision(), self.selection.head));
    }

    pub(super) fn yank(&mut self, _: &Yank, _: &mut Window, cx: &mut Context<Self>) {
        let text = cx.try_global::<Killed>().map(|k| k.0.clone()).unwrap_or_default();
        if text.is_empty() {
            return;
        }
        self.for_each_cursor(cx, |this, cx| this.edit(this.selection.range(), &text, EditKind::Other, cx));
    }

    /// Swaps the letters on each side of the caret and moves past them; at the end of a line,
    /// the last two.
    pub(super) fn transpose(&mut self, _: &Transpose, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            if !this.selection.is_empty() {
                return;
            }
            let caret = this.selection.head;
            let (line, column) = this.buffer.point(caret);
            // Two letters of the caret's own line (an empty line has none to swap).
            if this.buffer.line_len(line) < 2 {
                return;
            }
            let at = if column < this.buffer.line_len(line) { caret } else { caret.saturating_sub(1) };
            if this.buffer.point(at).1 == 0 {
                return;
            }
            let (Some(a), Some(b)) = (this.buffer.char_at(at - 1), this.buffer.char_at(at)) else { return };
            this.edit(at - 1..at + 1, &format!("{b}{a}"), EditKind::Other, cx);
            this.selection = Selection::caret(at + 1);
        });
    }

    /// A line break after the caret, the caret staying where it is.
    pub(super) fn open_line(&mut self, _: &OpenLine, _: &mut Window, cx: &mut Context<Self>) {
        self.for_each_cursor(cx, |this, cx| {
            let start = this.selection.range().start;
            let ending = this.style.line_ending.text();
            this.edit(this.selection.range(), ending, EditKind::Other, cx);
            this.selection = Selection::caret(start);
        });
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use gpui::{Focusable, TestAppContext};

    #[gpui::test]
    fn the_control_keys_of_mac_text_fields(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(crate::buffer::Buffer::from_text("one two\nthree\nfour\n"), Some("x.txt".into()), cx)
        });
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            e.selection = Selection::caret(4);
        });
        let state = |cx: &mut gpui::VisualTestContext| e.update(cx, |e, _| (e.buffer.to_string(), e.selection.head));
        cx.simulate_keystrokes("ctrl-e");
        assert_eq!(state(cx), ("one two\nthree\nfour\n".into(), 7));
        cx.simulate_keystrokes("ctrl-a ctrl-f ctrl-f ctrl-t");
        assert_eq!(state(cx), ("oen two\nthree\nfour\n".into(), 3));
        // ⌃K twice takes the rest of the line, then its break; ⌃Y puts both back.
        cx.simulate_keystrokes("ctrl-k ctrl-k");
        assert_eq!(state(cx), ("oenthree\nfour\n".into(), 3));
        cx.simulate_keystrokes("ctrl-n ctrl-e ctrl-y");
        assert_eq!(state(cx), ("oenthree\nfour two\n\n".into(), 18));
        cx.simulate_keystrokes("ctrl-p ctrl-a ctrl-o");
        assert_eq!(state(cx), ("oenthree\n\nfour two\n\n".into(), 9));
        // ⌃T on an empty line: nothing to swap.
        e.update(cx, |e, _| e.selection = Selection::caret(e.buffer.len_chars()));
        let before = state(cx);
        cx.simulate_keystrokes("ctrl-t");
        assert_eq!(state(cx), before);
        e.update(cx, |e, _| e.selection = Selection::caret(9));
        cx.simulate_keystrokes("ctrl-d ctrl-f ctrl-h");
        assert_eq!(state(cx), ("oenthree\nour two\n\n".into(), 9));
    }
}

#[cfg(test)]
mod subword_tests {
    use super::{subword_left, subword_right};

    /// Every stop going right, then going left, through `text`.
    fn stops(text: &str) -> (Vec<usize>, Vec<usize>) {
        let chars: Vec<char> = text.chars().collect();
        let (mut right, mut at) = (Vec::new(), 0);
        while at < chars.len() {
            at = subword_right(&chars, at);
            right.push(at);
        }
        let (mut left, mut at) = (Vec::new(), chars.len());
        while at > 0 {
            at = subword_left(&chars, at);
            left.push(at);
        }
        (right, left)
    }

    #[test]
    fn names_go_by_their_parts() {
        assert_eq!(stops("parseHttpRequest"), (vec![5, 9, 16], vec![9, 5, 0]));
        assert_eq!(stops("snake_case_name"), (vec![5, 10, 15], vec![11, 6, 0]));
        assert_eq!(stops("HTMLParser"), (vec![4, 10], vec![4, 0]));
        assert_eq!(stops("x = getID(v2)"), (vec![1, 3, 7, 9, 10, 11, 12, 13], vec![12, 11, 10, 9, 7, 4, 2, 0]));
        assert_eq!(stops("été_très"), (vec![3, 8], vec![4, 0]));
    }
}

#[cfg(test)]
mod view_tests {
    use crate::buffer::Buffer;
    use crate::editor::{Editor, Selection};
    use gpui::TestAppContext;
    use std::path::PathBuf;

    /// ⌃⌥→ twice, ⌃⌥⇧← to select a part, ⌃⌥⌫ to delete one: by the parts of a name.
    #[gpui::test]
    fn keys_go_by_the_parts_of_a_name(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(Buffer::from_text("let parseHttpRequest = 1;\n"), Some(PathBuf::from("a.rs")), cx)
        });
        e.update_in(cx, |e, window, _| {
            window.focus(&e.focus_handle);
            e.selection = Selection::caret(4);
        });
        cx.simulate_keystrokes("ctrl-alt-right ctrl-alt-right");
        e.update(cx, |e, _| assert_eq!(e.selection.head, 13, "after `parseHttp`"));
        cx.simulate_keystrokes("ctrl-alt-shift-left");
        e.update(cx, |e, _| assert_eq!(e.buffer.slice(e.selection.range()), "Http"));
        e.update(cx, |e, _| e.selection = Selection::caret(13));
        cx.simulate_keystrokes("ctrl-alt-backspace");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "let parseRequest = 1;\n"));
    }

    /// ⌃L puts the caret's line in the middle; ⌘J, after scrolling away, brings it back.
    #[gpui::test]
    fn the_caret_line_centered_and_found_again(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let text = (1..=300).map(|n| format!("line {n}\n")).collect::<String>();
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(PathBuf::from("a.txt")), cx));
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle);
            e.go_to_line(150, cx);
        });
        cx.run_until_parked();
        // Where the caret's line is in the view, as a share of its height (0 top, 1 bottom).
        let place = |cx: &mut gpui::VisualTestContext| {
            e.read_with(cx, |e, _| {
                let layout = e.layout.as_ref().unwrap();
                let height = f32::from(layout.text_bounds.size.height);
                let row = e.caret_point().0 as f32;
                let y = row * f32::from(layout.line_height) - e.scroll.target_y;
                y / height
            })
        };
        assert!(place(cx) > 0.6, "near the bottom after going there: {}", place(cx));
        cx.simulate_keystrokes("ctrl-l");
        cx.run_until_parked();
        assert!((place(cx) - 0.5).abs() < 0.1, "in the middle: {}", place(cx));
        // Scrolled to the top, away from the caret; ⌘J brings it back.
        e.update(cx, |e, _| {
            e.scroll.target_y = 0.;
            e.scroll.y = 0.;
        });
        cx.run_until_parked();
        assert!(place(cx) > 1., "out of view");
        cx.simulate_keystrokes("cmd-j");
        cx.run_until_parked();
        assert!((0. ..1.).contains(&place(cx)), "in view: {}", place(cx));
        e.update(cx, |e, _| assert_eq!(e.selection, Selection::caret(e.buffer.offset(149, 0))));
    }
}
