//! Breakpoints: lines where the debugger stops, set with a click in the gutter (left of
//! the fold chevrons) or F9, and moved with the code as it's edited.

use super::{Editor, EditorEvent};
use gpui::{App, Context, KeyBinding, Pixels, Point, Window, actions};

actions!(debugging, [ToggleBreakpoint]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("f9", ToggleBreakpoint, Some("Editor"))]);
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
        }
        self.breakpoints.dedup();
        self.breakpoints_revision = revision;
        if self.breakpoints != before {
            cx.emit(EditorEvent::BreakpointsChanged);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
