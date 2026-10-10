//! Breakpoints: lines where the debugger stops, set with a click in the gutter (left of
//! the fold chevrons) or F9, and moved with the code as it's edited.

use super::{Editor, EditorEvent};
use crate::fonts::CodeFont;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use gpui::{
    AnyElement, App, Context, Entity, Focusable, KeyBinding, MouseDownEvent, Pixels, Point, Subscription, Window,
    actions, anchored, deferred, div, point, prelude::*, px,
};

actions!(debugging, [ToggleBreakpoint, ConfirmCondition, CancelCondition]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("f9", ToggleBreakpoint, Some("Editor")),
        KeyBinding::new("enter", ConfirmCondition, Some("ConditionField")),
        KeyBinding::new("escape", CancelCondition, Some("ConditionField")),
    ]);
}

/// What a breakpoint's field says, read: a condition it stops on ("i == 3"), the time it's
/// reached from which it stops ("5", ">= 5", "> 4": the 5th time and every time after, as
/// lldb counts), or a message it prints without stopping ("log x is {x}").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakWhen<'a> {
    Condition(&'a str),
    Hit(usize),
    Log(&'a str),
}

impl<'a> BreakWhen<'a> {
    pub fn read(text: &'a str) -> Self {
        let text = text.trim();
        // "log x is {x}" (not "log == 3": a variable called log).
        if let Some(message) = text.strip_prefix("log:").or_else(|| text.strip_prefix("log ")) {
            let message = message.trim();
            let operator = message.starts_with(['=', '!', '<', '>', '.', '-', '(', '[', '&', '|', '+', '*', '/', '%']);
            if !operator {
                // Nothing to say: a plain breakpoint.
                return if message.is_empty() { BreakWhen::Condition("") } else { BreakWhen::Log(message) };
            }
        }
        // A count: digits, maybe after >=, == or > (one more).
        let (after, rest) = match text {
            t if t.starts_with(">=") || t.starts_with("==") => (0, &t[2..]),
            t if t.starts_with('>') => (1, &t[1..]),
            t => (0, t),
        };
        let digits = rest.trim();
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            let n: usize = digits.parse().unwrap_or(usize::MAX - 1);
            return BreakWhen::Hit((n + after).max(1));
        }
        BreakWhen::Condition(text)
    }
}

/// A breakpoint: its line (from 0), and when it only stops if something holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Breakpoint {
    pub line: usize,
    pub condition: Option<String>,
}

/// The field a breakpoint's condition is typed in, beside it.
pub(super) struct ConditionEdit {
    line: usize,
    input: Entity<TextInput>,
    _subscription: Subscription,
}

/// Where a mark on line `line` (a breakpoint, a bookmark) goes after `edit`: along with
/// lines below it, and with its own line when the edit ends right at its start (Enter there,
/// lines above taken away); on the edit's first line if its own line was edited away.
pub(super) fn move_line(line: usize, edit: &crate::buffer::Edit) -> usize {
    let (start, old_end, new_end) = (edit.start.0, edit.old_end.0, edit.new_end.0);
    if edit.old_end == (line, 0) {
        line - old_end + new_end
    } else if line < start {
        line
    } else if line > old_end {
        line - old_end + new_end
    } else {
        start.max(line.min(new_end))
    }
}

impl Editor {
    pub(super) fn toggle_breakpoint(&mut self, _: &ToggleBreakpoint, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.buffer.point(self.selection.head).0;
        self.toggle_breakpoint_at(line, cx);
    }

    pub(super) fn toggle_breakpoint_at(&mut self, line: usize, cx: &mut Context<Self>) {
        match self.breakpoints.binary_search(&line) {
            Ok(i) => {
                self.breakpoints.remove(i);
                self.breakpoint_conditions.retain(|(l, _)| *l != line);
            }
            Err(i) => self.breakpoints.insert(i, line),
        }
        self.breakpoints_revision = self.buffer.revision();
        cx.emit(EditorEvent::BreakpointsChanged);
        cx.notify();
    }

    /// The line a click in the gutter, on or left of the line numbers, sets a breakpoint on.
    pub(super) fn breakpoint_click(&self, position: Point<Pixels>) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let numbers_end =
            layout.text_bounds.left() - gpui::px(crate::element::GUTTER_PADDING + crate::element::FOLD_SPACE);
        if !layout.bounds.contains(&position) || position.x >= numbers_end {
            return None;
        }
        let row = ((position.y - layout.text_origin.y) / layout.line_height).floor();
        let r = layout.rows.get((row as usize).checked_sub(layout.first_row)?)?;
        r.row.block.is_none().then_some(r.row.line)
    }

    /// After an edit: the breakpoints move with their lines.
    pub(super) fn breakpoints_after_edit(&mut self, cx: &mut Context<Self>) {
        let revision = self.buffer.revision();
        if self.breakpoints.is_empty() {
            self.breakpoints_revision = revision;
            return;
        }
        let Some(edits) = self.buffer.edits_since(self.breakpoints_revision) else {
            self.breakpoints_revision = revision;
            return;
        };
        let before = self.breakpoints.clone();
        for edit in edits {
            for line in &mut self.breakpoints {
                *line = move_line(*line, edit);
            }
            for (line, _) in &mut self.breakpoint_conditions {
                *line = move_line(*line, edit);
            }
        }
        self.breakpoints.dedup();
        self.breakpoint_conditions.dedup_by_key(|(l, _)| *l);
        self.breakpoints_revision = revision;
        if self.breakpoints != before {
            cx.emit(EditorEvent::BreakpointsChanged);
        }
    }
}

impl Editor {
    /// The breakpoints with their conditions, for the debugger.
    pub fn breakpoint_list(&self) -> Vec<Breakpoint> {
        self.breakpoints
            .iter()
            .map(|&line| Breakpoint {
                line,
                condition: self.breakpoint_conditions.iter().find(|(l, _)| *l == line).map(|(_, c)| c.clone()),
            })
            .collect()
    }

    /// Right-click in the breakpoint strip: a field to make that line's breakpoint stop
    /// only when something holds.
    pub(super) fn on_right_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // In the text: what can be done there (see `show_text_menu`).
        let Some(line) = self.breakpoint_click(event.position) else { return self.show_text_menu(event, cx) };
        let current = self.breakpoint_conditions.iter().find(|(l, _)| *l == line).map(|(_, c)| c.clone());
        let input = cx.new(|cx| {
            let mut input =
                TextInput::new("Stop when i == 3 · 5: from the 5th time on · log x is {x}: print, don't stop", cx);
            if let Some(text) = &current {
                input.set_text(text, cx);
            }
            input
        });
        let subscription = cx.subscribe(&input, |_, _, TextInputEvent::Changed, cx| cx.notify());
        window.focus(&input.focus_handle(cx));
        // Otherwise the click goes on to focus the editor it landed in, taking the keyboard
        // back from the field: what was typed went into the code.
        window.prevent_default();
        self.editing_condition = Some(ConditionEdit { line, input, _subscription: subscription });
        cx.notify();
    }

    fn confirm_condition(&mut self, _: &ConfirmCondition, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.editing_condition.take() else { return };
        let text = edit.input.read(cx).text().trim().to_string();
        self.breakpoint_conditions.retain(|(l, _)| *l != edit.line);
        if !text.is_empty() {
            if let Err(i) = self.breakpoints.binary_search(&edit.line) {
                self.breakpoints.insert(i, edit.line);
            }
            self.breakpoint_conditions.push((edit.line, text));
            self.breakpoint_conditions.sort_by_key(|(l, _)| *l);
        }
        self.breakpoints_revision = self.buffer.revision();
        window.focus(&self.focus_handle);
        cx.emit(EditorEvent::BreakpointsChanged);
        cx.notify();
    }

    fn cancel_condition(&mut self, _: &CancelCondition, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_condition = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// The condition field, on the breakpoint's line just past the gutter.
    pub(super) fn render_condition(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let edit = self.editing_condition.as_ref()?;
        let layout = self.layout.as_ref()?;
        let row = self.wrap.first_row(edit.line);
        let y = layout.text_origin.y + layout.line_height * row as f32;
        let theme = cx.global::<Theme>();
        Some(
            deferred(
                anchored().position(point(layout.text_bounds.left(), y - px(3.))).child(
                    div()
                        .key_context("ConditionField")
                        .on_action(cx.listener(Self::confirm_condition))
                        .on_action(cx.listener(Self::cancel_condition))
                        .occlude()
                        .w(layout.char_width * 40.)
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(theme.error.opacity(0.6))
                        .bg(theme.raised)
                        .shadow_md()
                        .code_font(cx)
                        .text_size(self.font_size)
                        .line_height(self.line_height())
                        .child(edit.input.clone()),
                ),
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_breakpoint_s_field_is_read() {
        assert_eq!(BreakWhen::read(" i == 3 "), BreakWhen::Condition("i == 3"));
        assert_eq!(BreakWhen::read("5"), BreakWhen::Hit(5));
        assert_eq!(BreakWhen::read(">= 12"), BreakWhen::Hit(12));
        assert_eq!(BreakWhen::read("> 4"), BreakWhen::Hit(5), "past the 4th: from the 5th");
        assert_eq!(BreakWhen::read("0"), BreakWhen::Hit(1));
        assert_eq!(BreakWhen::read("5 > i"), BreakWhen::Condition("5 > i"));
        assert_eq!(BreakWhen::read("log total is {total}"), BreakWhen::Log("total is {total}"));
        assert_eq!(BreakWhen::read("log: hi"), BreakWhen::Log("hi"));
        assert_eq!(BreakWhen::read("log:"), BreakWhen::Condition(""), "nothing to say: plain");
        assert_eq!(BreakWhen::read("log == 3"), BreakWhen::Condition("log == 3"), "a variable called log");
        assert_eq!(BreakWhen::read("logged == 3"), BreakWhen::Condition("logged == 3"), "a name starting with log");
    }

    /// With the mouse: a right-click in the strip left of the line numbers opens the
    /// condition field; typed into and ↵, that line's breakpoint stops only then.
    #[gpui::test]
    fn right_clicking_the_gutter_sets_a_condition(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(Buffer::from_text("for i in 0..9 {\n    work(i);\n}\n"), Some("x.rs".into()), cx)
        });
        e.update_in(cx, |e, window, cx| window.focus(&e.focus_handle(cx)));
        cx.run_until_parked();
        let in_strip = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().expect("drawn");
            gpui::point(l.bounds.left() + gpui::px(8.), l.text_origin.y + l.line_height * 1.5)
        });
        cx.simulate_event(gpui::MouseDownEvent {
            position: in_strip,
            button: gpui::MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.run_until_parked();
        assert!(e.read_with(cx, |e, _| e.editing_condition.is_some()), "no field opened");
        cx.simulate_input("i == 3");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(
            e.read_with(cx, |e, _| e.breakpoint_list()),
            [Breakpoint { line: 1, condition: Some("i == 3".into()) }]
        );
    }

    #[gpui::test]
    fn conditions_go_with_their_breakpoints(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        use gpui::AppContext as _;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let e = cx.new(|cx| Editor::new(Buffer::from_text("a\nb\nc\n"), Some(std::path::PathBuf::from("x.c")), cx));
        e.update(cx, |e, cx| {
            e.breakpoints = vec![1, 2];
            e.breakpoint_conditions = vec![(2, "i == 3".into())];
            // A line added at the top: both move down, the condition with its breakpoint.
            e.type_text_for_test(0, "top\n", cx);
            assert_eq!(
                e.breakpoint_list(),
                [Breakpoint { line: 2, condition: None }, Breakpoint { line: 3, condition: Some("i == 3".into()) }]
            );
            // Removing the breakpoint takes its condition away.
            e.toggle_breakpoint_at(3, cx);
            assert!(e.breakpoint_conditions.is_empty());
        });
    }

    #[test]
    fn breakpoints_move_with_their_lines() {
        let edit = |start: (usize, usize), old_end: (usize, usize), new_end: (usize, usize)| crate::buffer::Edit {
            start_byte: 0,
            old_end_byte: 0,
            new_end_byte: 0,
            start,
            old_end,
            new_end,
            lsp_range: None,
            text: String::new(),
        };
        // Two lines added above line 5 (an edit on line 1 now ending on line 3).
        assert_eq!(move_line(5, &edit((1, 4), (1, 4), (3, 0))), 7);
        // Lines below an edit after them don't move.
        assert_eq!(move_line(5, &edit((8, 0), (8, 2), (9, 1))), 5);
        // Its own line joined into the one above: it goes there.
        assert_eq!(move_line(5, &edit((4, 6), (5, 0), (4, 6))), 4);
        assert_eq!(move_line(5, &edit((4, 6), (5, 3), (4, 6))), 4);
        // Typing on its line keeps it there.
        assert_eq!(move_line(5, &edit((5, 2), (5, 2), (5, 3))), 5);
        // Enter at the very start of its line: it goes down with its text.
        assert_eq!(move_line(5, &edit((5, 0), (5, 0), (6, 0))), 6);
        // The lines above it, up to its start, taken away: it comes up with its text.
        assert_eq!(move_line(5, &edit((2, 0), (5, 0), (2, 0))), 2);
    }
}
