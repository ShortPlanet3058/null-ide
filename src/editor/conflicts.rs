//! Merge conflicts: the blocks git leaves between `<<<<<<<` and `>>>>>>>`. Each side is
//! tinted faintly, ⌘. offers to keep one side or both, and ⌥F8 goes from one to the next.

use super::fixes::FixMenu;
use super::{EditKind, Editor, Selection};
use gpui::{App, Context, KeyBinding, ScrollHandle, Window, actions};
use lsp_types::{CodeActionOrCommand, Command};
use ropey::{Rope, RopeSlice};
use std::ops::Range;
use std::rc::Rc;

actions!(conflicts, [NextConflict, PreviousConflict]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Editor");
    cx.bind_keys([
        KeyBinding::new("alt-f8", NextConflict, ctx),
        KeyBinding::new("alt-shift-f8", PreviousConflict, ctx),
    ]);
}

/// One conflict, by the lines of its markers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// `<<<<<<<`: the current side (HEAD) follows.
    pub start: usize,
    /// `|||||||`, in the diff3 style: the common ancestor follows.
    pub base: Option<usize>,
    /// `=======`: the incoming side follows.
    pub middle: usize,
    /// `>>>>>>>`.
    pub end: usize,
}

impl Conflict {
    pub fn current(&self) -> Range<usize> {
        self.start + 1..self.base.unwrap_or(self.middle)
    }

    pub fn incoming(&self) -> Range<usize> {
        self.middle + 1..self.end
    }

    pub fn lines(&self) -> Range<usize> {
        self.start..self.end + 1
    }
}

/// The choices ⌘. offers in a conflict, in the order listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Keep {
    Current,
    Incoming,
    Both,
}

const CHOICES: [Keep; 3] = [Keep::Current, Keep::Incoming, Keep::Both];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Marker {
    Start,
    Base,
    Middle,
    End,
}

/// What marker a line is: seven of `<`, `|`, `=` or `>` at its start, then a space and a
/// label or nothing (only nothing for `=======`).
fn marker(line: RopeSlice) -> Option<Marker> {
    let mut chars = line.chars();
    let first = chars.next()?;
    let kind = match first {
        '<' => Marker::Start,
        '|' => Marker::Base,
        '=' => Marker::Middle,
        '>' => Marker::End,
        _ => return None,
    };
    for _ in 1..7 {
        if chars.next()? != first {
            return None;
        }
    }
    match chars.next() {
        None | Some('\n' | '\r') => Some(kind),
        Some(' ') if kind != Marker::Middle => Some(kind),
        _ => None,
    }
}

/// Whether `<<<<<<<` appears at all, without going line by line: most files have no
/// conflict, and this runs after every edit.
fn might_have_conflicts(rope: &Rope) -> bool {
    // Seven in a row can straddle two chunks: count the `<` that end the last one.
    let mut trailing = 0;
    for chunk in rope.chunks() {
        if chunk.contains("<<<<<<<") {
            return true;
        }
        let leading = chunk.bytes().take_while(|&b| b == b'<').count();
        if trailing + leading >= 7 {
            return true;
        }
        trailing = if leading == chunk.len() {
            trailing + leading
        } else {
            chunk.bytes().rev().take_while(|&b| b == b'<').count()
        };
    }
    false
}

/// Every complete conflict in the text, in order. A marker out of place (a lone `=======`,
/// as in Markdown) starts nothing.
pub fn find(rope: &Rope) -> Vec<Conflict> {
    if !might_have_conflicts(rope) {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut open: Option<Conflict> = None;
    for (ix, line) in rope.lines().enumerate() {
        let Some(kind) = marker(line) else { continue };
        match (kind, &mut open) {
            (Marker::Start, _) => open = Some(Conflict { start: ix, base: None, middle: usize::MAX, end: 0 }),
            (Marker::Base, Some(c)) if c.base.is_none() && c.middle == usize::MAX => c.base = Some(ix),
            (Marker::Middle, Some(c)) if c.middle == usize::MAX => c.middle = ix,
            (Marker::End, Some(c)) if c.middle != usize::MAX => {
                c.end = ix;
                found.extend(open.take());
            }
            _ => {}
        }
    }
    found
}

/// The label after a marker ("HEAD", "feature/login"…), shortened for the list.
fn label(line: &str) -> Option<String> {
    let label = line.get(8..)?.trim();
    if label.is_empty() {
        return None;
    }
    const MAX: usize = 32;
    Some(if label.chars().count() > MAX {
        format!("{}…", label.chars().take(MAX - 1).collect::<String>())
    } else {
        label.to_string()
    })
}

impl Editor {
    /// The file's conflicts, found again only after the text changes.
    pub fn conflicts(&self) -> Rc<[Conflict]> {
        let mut known = self.conflicts.borrow_mut();
        let revision = self.buffer.revision();
        if known.0 != revision {
            // No conflicts before, and no marker on the lines edited since: still none.
            // Looking through the whole file again took most of a keystroke on a big log.
            let rope = self.buffer.rope();
            let last = rope.len_lines().saturating_sub(1);
            let still_none = known.1.is_empty()
                && known.0 != u64::MAX
                && self.buffer.edits_since(known.0).is_some_and(|mut edits| {
                    edits.all(|e| (e.start.0..=e.new_end.0).all(|l| marker(rope.line(l.min(last))).is_none()))
                });
            *known = (revision, if still_none { known.1.clone() } else { find(rope).into() });
        }
        known.1.clone()
    }

    /// The conflict the caret is in, markers included.
    pub(super) fn conflict_at_caret(&self) -> Option<Conflict> {
        let line = self.buffer.point(self.selection.head).0;
        self.conflicts().iter().find(|c| c.lines().contains(&line)).copied()
    }

    /// ⌘. inside a conflict: its choices, in the quick-fix list, without asking a server.
    pub(super) fn conflict_choices(&mut self, conflict: Conflict, cx: &mut Context<Self>) {
        let side = |line: usize, plain: &str| match label(&self.buffer.line_text(line)) {
            Some(label) => format!("Keep {plain} ({label})"),
            None => format!("Keep {plain}"),
        };
        let titles = [side(conflict.start, "current"), side(conflict.end, "incoming"), "Keep both".to_string()];
        let fixes = titles
            .into_iter()
            .map(|title| CodeActionOrCommand::Command(Command { title, command: String::new(), arguments: None }))
            .collect();
        self.close_completion(cx);
        self.close_hover(cx);
        // Hung from the end of the first marker, under its hint, off the code it chooses between.
        let at = self.buffer.line_to_char(conflict.start) + self.buffer.line_len(conflict.start);
        self.fix_menu = Some(FixMenu {
            fixes,
            selected: 0,
            at,
            scroll: ScrollHandle::new(),
            conflict: Some(conflict),
            spelling: None,
        });
        cx.notify();
    }

    /// Replaces the conflict with the side (or sides) picked from its list.
    pub(super) fn resolve_conflict(&mut self, conflict: Conflict, choice: usize, cx: &mut Context<Self>) {
        let Some(&keep) = CHOICES.get(choice) else { return };
        // The text may have changed under the list: only resolve the conflict still there.
        if !self.conflicts().contains(&conflict) {
            return;
        }
        let kept: Vec<usize> = match keep {
            Keep::Current => conflict.current().collect(),
            Keep::Incoming => conflict.incoming().collect(),
            Keep::Both => conflict.current().chain(conflict.incoming()).collect(),
        };
        let start = self.buffer.line_to_char(conflict.start);
        if kept.is_empty() {
            // Nothing kept: the lines go, their line breaks too.
            let range = self.lines_char_range(&conflict.lines());
            self.edit(range, "", EditKind::Other, cx);
        } else {
            let texts = kept.into_iter().map(|l| self.buffer.line_text(l)).collect();
            self.rewrite_lines(conflict.lines(), texts, |_| (conflict.start, 0), cx);
        }
        self.single_cursor();
        self.selection = Selection::caret(start.min(self.buffer.len_chars()));
        self.touch(cx);
        if self.conflicts().is_empty() {
            self.show_notice(self.selection.head, "No conflicts left in this file.".into(), cx);
        }
    }

    pub(super) fn next_conflict(&mut self, _: &NextConflict, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_conflict(true, cx);
    }

    pub(super) fn previous_conflict(&mut self, _: &PreviousConflict, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to_conflict(false, cx);
    }

    /// To the next conflict's first marker (round to the first).
    pub fn go_to_conflict(&mut self, forward: bool, cx: &mut Context<Self>) {
        let line = self.buffer.point(self.selection.head).0;
        let conflicts = self.conflicts();
        let target = if forward {
            conflicts.iter().find(|c| c.start > line).or(conflicts.first())
        } else {
            conflicts.iter().rev().find(|c| c.start < line).or(conflicts.last())
        };
        let Some(target) = target.map(|c| c.start) else {
            let at = self.selection.head;
            return self.show_notice(at, "No conflicts in this file.".into(), cx);
        };
        cx.emit(super::EditorEvent::Jumped { from: self.caret_point() });
        self.single_cursor();
        self.selection = Selection::caret(self.buffer.line_to_char(target));
        self.goal_column = None;
        self.touch(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use gpui::TestAppContext;
    use std::path::PathBuf;

    fn editor<'a>(cx: &'a mut TestAppContext, text: &str) -> (gpui::Entity<Editor>, &'a mut gpui::VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("x.txt")), cx))
    }

    const TWO: &str =
        "a\n<<<<<<< HEAD\nmine\n=======\ntheirs\n>>>>>>> feature\nb\n<<<<<<< HEAD\n=======\nnew\n>>>>>>> feature\n";

    #[gpui::test]
    fn quick_fix_offers_and_keeps_a_side(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, TWO);
        e.update_in(cx, |e, window, cx| {
            e.set_caret_point((2, 1), cx);
            e.quick_fix(&super::super::QuickFix, window, cx);
            let menu = e.fix_menu.as_ref().expect("the conflict's choices");
            let titles: Vec<&str> = menu.fixes.iter().map(super::super::fixes::title).collect();
            assert_eq!(titles, ["Keep current (HEAD)", "Keep incoming (feature)", "Keep both"]);
            e.accept_fix(2, window, cx);
            assert_eq!(e.buffer.to_string(), "a\nmine\ntheirs\nb\n<<<<<<< HEAD\n=======\nnew\n>>>>>>> feature\n");
            assert_eq!(e.caret_point(), (1, 0));
            // An empty side kept: the conflict's lines go entirely.
            e.go_to_conflict(true, cx);
            assert_eq!(e.caret_point(), (4, 0));
            e.quick_fix(&super::super::QuickFix, window, cx);
            e.accept_fix(0, window, cx);
            assert_eq!(e.buffer.to_string(), "a\nmine\ntheirs\nb\n");
            assert!(e.conflicts().is_empty());
            // Undo brings the conflict back in one step.
            e.undo(&super::super::Undo, window, cx);
            assert_eq!(e.conflicts().len(), 1);
        });
    }

    #[gpui::test]
    fn conflicts_are_visited_in_turn(cx: &mut TestAppContext) {
        let (e, cx) = editor(cx, TWO);
        e.update_in(cx, |e, window, cx| {
            e.next_conflict(&NextConflict, window, cx);
            assert_eq!(e.caret_point(), (1, 0));
            e.next_conflict(&NextConflict, window, cx);
            assert_eq!(e.caret_point(), (7, 0));
            e.next_conflict(&NextConflict, window, cx);
            assert_eq!(e.caret_point(), (1, 0));
            e.previous_conflict(&PreviousConflict, window, cx);
            assert_eq!(e.caret_point(), (7, 0));
        });
    }

    #[test]
    fn finds_conflicts_and_their_sides() {
        let text = "a\n<<<<<<< HEAD\nmine\n=======\ntheirs\n>>>>>>> feature\nb\n\
                    <<<<<<< HEAD\nx\n||||||| base\nold\n=======\n>>>>>>> other\n";
        let found = find(&Rope::from_str(text));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], Conflict { start: 1, base: None, middle: 3, end: 5 });
        assert_eq!((found[0].current(), found[0].incoming()), (2..3, 4..5));
        assert_eq!(found[1].base, Some(9));
        assert_eq!((found[1].current(), found[1].incoming()), (8..9, 12..12));
    }

    #[test]
    fn leaves_lookalikes_alone() {
        // A Markdown heading underline, and markers with something else after them.
        assert!(find(&Rope::from_str("Title\n=======\n")).is_empty());
        assert!(find(&Rope::from_str("<<<<<<<< eight\n=======\n>>>>>>> x\n")).is_empty());
        assert!(find(&Rope::from_str("<<<<<<< HEAD\nonly the start\n")).is_empty());
        assert_eq!(label("<<<<<<< HEAD").as_deref(), Some("HEAD"));
        assert_eq!(label("======="), None);
    }

    #[test]
    fn spots_markers_across_chunks() {
        // Long enough for several chunks, with the marker wherever they split.
        for pad in 0..40 {
            let text = format!("{}\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> x\n", "y".repeat(1000 + pad * 37));
            assert_eq!(find(&Rope::from_str(&text)).len(), 1, "pad {pad}");
        }
    }
}
