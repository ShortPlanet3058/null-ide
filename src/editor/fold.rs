//! Folding: a block's inner lines hidden behind its first line, which ends in "⋯". The
//! closing line stays, so the shape of the code still shows. Regions come from the
//! syntax tree; a fold opens again as soon as the caret goes inside it.

use super::Editor;
use gpui::{App, Context, KeyBinding, Window, actions};
use std::ops::Range;

actions!(fold, [Fold, Unfold, FoldAll, UnfoldAll]);

pub fn bind_keys(cx: &mut App) {
    let editor = Some("Editor");
    // As in Xcode: arrows, so they work on every keyboard layout (unlike ⌘[ and ⌘]).
    cx.bind_keys([
        KeyBinding::new("alt-secondary-left", Fold, editor),
        KeyBinding::new("alt-secondary-right", Unfold, editor),
    ]);
}

#[derive(Default)]
pub(super) struct Folds {
    /// Folded regions, as lines from the opening one to the closing one. Sorted.
    pub folded: Vec<Range<usize>>,
    /// The buffer revision `folded` matches.
    revision: u64,
    /// What can fold, for a buffer revision.
    foldable: Option<(u64, Vec<Range<usize>>)>,
}

impl Editor {
    /// The blocks around `line` (start above it, end below it), innermost first. Cheap:
    /// a walk up the syntax tree from the line.
    pub fn blocks_around(&mut self, line: usize) -> Vec<Range<usize>> {
        let Some(highlighter) = &mut self.highlighter else { return Vec::new() };
        highlighter.sync(&self.buffer);
        highlighter.blocks_around(line)
    }

    /// The regions that can fold, worked out again only after edits.
    pub fn foldable(&mut self) -> &[Range<usize>] {
        let revision = self.buffer.revision();
        if self.folds.foldable.as_ref().is_none_or(|(r, _)| *r != revision) {
            let ranges = match &mut self.highlighter {
                Some(highlighter) => {
                    highlighter.sync(&self.buffer);
                    highlighter.fold_ranges()
                }
                None => Vec::new(),
            };
            self.folds.foldable = Some((revision, ranges));
        }
        &self.folds.foldable.as_ref().unwrap().1
    }

    /// The folded regions, as (first line, last line), to save with the session.
    pub fn folded_regions(&self) -> Vec<(usize, usize)> {
        self.folds.folded.iter().map(|r| (r.start, r.end)).collect()
    }

    /// Folds these regions again (from a session), skipping any the text no longer has.
    pub fn restore_folds(&mut self, regions: &[(usize, usize)], cx: &mut Context<Self>) {
        let lines = self.buffer.len_lines();
        self.folds.folded = regions.iter().filter(|(a, b)| a + 1 < *b && *b < lines).map(|&(a, b)| a..b).collect();
        self.folds.folded.sort_by_key(|r| r.start);
        self.apply_folds(cx);
    }

    pub fn is_folded(&self, line: usize) -> bool {
        self.folds.folded.iter().any(|r| r.start == line)
    }

    /// The region `line` opens, or else the innermost one around it.
    fn region_at(&mut self, line: usize) -> Option<Range<usize>> {
        let foldable = self.foldable();
        foldable
            .iter()
            .find(|r| r.start == line)
            .cloned()
            .or_else(|| foldable.iter().filter(|r| r.start < line && line < r.end).min_by_key(|r| r.len()).cloned())
    }

    pub(super) fn fold(&mut self, _: &Fold, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.buffer.point(self.selection.head).0;
        let region = match self.region_at(line) {
            Some(region) if !self.folds.folded.contains(&region) => Some(region),
            // Already folded here: the region around it.
            Some(inner) => self
                .foldable()
                .iter()
                .filter(|r| r.start < inner.start && inner.end <= r.end)
                .min_by_key(|r| r.len())
                .cloned(),
            None => None,
        };
        if let Some(region) = region {
            self.set_folded(region, true, cx);
        }
    }

    pub(super) fn unfold(&mut self, _: &Unfold, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.buffer.point(self.selection.head).0;
        let before = self.folds.folded.len();
        self.folds.folded.retain(|r| !(r.start <= line && line <= r.end));
        if self.folds.folded.len() != before {
            self.apply_folds(cx);
        }
    }

    pub(super) fn fold_all(&mut self, _: &FoldAll, _: &mut Window, cx: &mut Context<Self>) {
        self.folds.folded = self.foldable().to_vec();
        self.fold_moves_caret_out();
        self.apply_folds(cx);
    }

    pub(super) fn unfold_all(&mut self, _: &UnfoldAll, _: &mut Window, cx: &mut Context<Self>) {
        self.folds.folded.clear();
        self.apply_folds(cx);
    }

    /// Clicking the gutter's chevron, or the "⋯".
    pub fn toggle_fold(&mut self, line: usize, cx: &mut Context<Self>) {
        if let Some(region) = self.folds.folded.iter().find(|r| r.start == line).cloned() {
            return self.set_folded(region, false, cx);
        }
        if let Some(region) = self.foldable().iter().find(|r| r.start == line).cloned() {
            self.set_folded(region, true, cx);
        }
    }

    fn set_folded(&mut self, region: Range<usize>, folded: bool, cx: &mut Context<Self>) {
        self.folds.folded.retain(|r| *r != region);
        if folded {
            let at = self.folds.folded.partition_point(|r| r.start < region.start);
            self.folds.folded.insert(at, region);
            self.fold_moves_caret_out();
        }
        self.apply_folds(cx);
    }

    /// A caret inside what was just folded moves to the end of the line that stays.
    fn fold_moves_caret_out(&mut self) {
        let line = self.buffer.point(self.selection.head).0;
        if let Some(region) = self.folds.folded.iter().find(|r| r.start < line && line < r.end) {
            let start = region.start;
            self.single_cursor();
            self.selection = super::Selection::caret(self.buffer.offset(start, usize::MAX));
        }
    }

    /// Tells the rows which lines are hidden.
    fn apply_folds(&mut self, cx: &mut Context<Self>) {
        self.folds.revision = self.buffer.revision();
        self.wrap.set_hidden(hidden_lines(&self.folds.folded));
        // Folding only keeps the caret in view; it doesn't pull the text about (typewriter
        // scrolling). A caret moved by the keyboard says otherwise right after, in `touch`.
        self.autoscroll = true;
        self.reveal_only = true;
        cx.notify();
    }

    /// After an edit: folds move with the lines around them. An edit inside a fold (or
    /// across its edges) opens it; one on its first or last line alone doesn't.
    pub(super) fn folds_after_edit(&mut self, cx: &mut Context<Self>) {
        if self.folds.folded.is_empty() {
            self.folds.revision = self.buffer.revision();
            return;
        }
        let Some(edits) = self.buffer.edits_since(self.folds.revision) else {
            // Undo replaced the text: start over unfolded.
            self.folds.folded.clear();
            return self.apply_folds(cx);
        };
        let mut folded = std::mem::take(&mut self.folds.folded);
        for edit in edits {
            let (from, to, new_to) = (edit.start.0, edit.old_end.0, edit.new_end.0);
            let shift = new_to as isize - to as isize;
            folded.retain_mut(|r| {
                // Above the fold, or right at the start of its first line (a line added above).
                if to < r.start || edit.old_end == (r.start, 0) {
                    r.start = (r.start as isize + shift) as usize;
                    r.end = (r.end as isize + shift) as usize;
                    true
                } else {
                    // Below it, nothing changed at its edges, or lines added at the very end of
                    // the closing line (they stay after the fold): kept as it is.
                    from > r.end
                        || (from == to && to == new_to && (from == r.start || from == r.end))
                        || (from == r.end && to == r.end)
                }
            });
        }
        self.folds.folded = folded;
        self.apply_folds(cx);
    }

    /// The caret went into folded lines (a search, a jump): open what hides it.
    pub(super) fn unfold_around_caret(&mut self, cx: &mut Context<Self>) {
        if self.folds.folded.is_empty() {
            return;
        }
        let lines: Vec<usize> = std::iter::once(self.selection.head)
            .chain(self.extra.iter().map(|c| c.selection.head))
            .map(|o| self.buffer.point(o).0)
            .collect();
        let before = self.folds.folded.len();
        self.folds.folded.retain(|r| !lines.iter().any(|&l| r.start < l && l < r.end));
        if self.folds.folded.len() != before {
            self.apply_folds(cx);
        }
    }
}

/// The lines folds hide: between each fold's first and last lines, merged.
fn hidden_lines(folded: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut hidden: Vec<Range<usize>> = Vec::new();
    for r in folded {
        let inner = r.start + 1..r.end;
        if inner.is_empty() {
            continue;
        }
        match hidden.last_mut() {
            Some(last) if inner.start <= last.end => last.end = last.end.max(inner.end),
            _ => hidden.push(inner),
        }
    }
    hidden
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_folds_hide_the_outer_lines_once() {
        assert_eq!(hidden_lines(&[0..10, 2..5, 12..14]), vec![1..10, 13..14]);
        assert_eq!(hidden_lines(&[3..4]), Vec::<Range<usize>>::new());
    }
}
