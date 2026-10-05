//! The ⌃ keys every macOS text field knows: ⌃A and ⌃E to the line's ends, ⌃F ⌃B ⌃N ⌃P to
//! move, ⌃D and ⌃H to delete, ⌃K to cut the rest of the line and ⌃Y to put it back, ⌃T to
//! swap two letters and ⌃O to open a line.

use super::{
    Backspace, CompletionNext, CompletionPrevious, Delete, EditKind, Editor, MoveDown, MoveLeft, MoveLineEnd,
    MoveLineStart, MoveRight, MoveUp, PageDown, SelectDown, SelectLeft, SelectLineEnd, SelectLineStart, SelectRight,
    SelectUp, Selection,
};
use gpui::{App, Context, Global, KeyBinding, Window, actions};

actions!(editor, [DeleteToLineEnd, Yank, Transpose, OpenLine]);

pub fn bind_keys(cx: &mut App) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let ctx = Some("Editor");
    cx.bind_keys([
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

impl Editor {
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
            this.edit(this.selection.range(), "\n", EditKind::Other, cx);
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
        cx.simulate_keystrokes("ctrl-d ctrl-f ctrl-h");
        assert_eq!(state(cx), ("oenthree\nour two\n\n".into(), 9));
    }
}
