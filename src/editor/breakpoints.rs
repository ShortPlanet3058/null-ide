//! Breakpoints: lines where the debugger stops, set with a click in the gutter (left of
//! the fold chevrons) or F9, and moved with the code as it's edited.

use super::{Editor, EditorEvent};
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

/// Where a breakpoint on line `line` goes after an edit from `start` to `old_end` (lines)
/// that now ends at `new_end`: along with lines below it; on the edit's first line if
/// its own line was edited away.
fn move_line(line: usize, start: usize, old_end: usize, new_end: usize) -> usize {
    if line < start {
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
            let (start, old_end, new_end) = (edit.start.0, edit.old_end.0, edit.new_end.0);
            for line in &mut self.breakpoints {
                *line = move_line(*line, start, old_end, new_end);
            }
            for (line, _) in &mut self.breakpoint_conditions {
                *line = move_line(*line, start, old_end, new_end);
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
        let Some(line) = self.breakpoint_click(event.position) else { return };
        let current = self.breakpoint_conditions.iter().find(|(l, _)| *l == line).map(|(_, c)| c.clone());
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Stop when… (like i == 3)", cx);
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
        let code_font = cx.global::<crate::fonts::Fonts>().code.clone();
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
                        .font_family(code_font)
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
        // Two lines added above line 5 (an edit on line 1 now ending on line 3).
        assert_eq!(move_line(5, 1, 1, 3), 7);
        // Lines below an edit after them don't move.
        assert_eq!(move_line(5, 8, 8, 9), 5);
        // Its own line joined into the one above: it goes there.
        assert_eq!(move_line(5, 4, 5, 4), 4);
        // Typing on its line keeps it there.
        assert_eq!(move_line(5, 5, 5, 5), 5);
    }
}
