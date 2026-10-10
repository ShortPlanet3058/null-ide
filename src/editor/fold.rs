//! Folding: a block's inner lines hidden behind its first line, which ends in "⋯". The
//! closing line stays, so the shape of the code still shows. Regions come from the
//! syntax tree, and runs of comment or import lines; lines selected fold too. A fold
//! opens again as soon as the caret goes inside it.

use super::Editor;
use gpui::{App, Context, KeyBinding, Window, actions};
use std::ops::Range;

actions!(fold, [Fold, Unfold, FoldAll, UnfoldAll, FoldLevel1, FoldLevel2, FoldLevel3]);

/// What starts a line that imports something, in the languages that do it line by line.
const IMPORTS: [&str; 7] = ["use ", "pub use ", "import ", "from ", "#include ", "#import ", "using "];
/// A run of comment or import lines this long or longer can fold.
const RUN: usize = 3;

#[derive(PartialEq, Clone, Copy, Debug)]
enum Kind {
    Comment,
    Import,
}

/// Whether a line (its text from the indentation on) is a comment (starting with
/// `comment`), an import, or neither.
fn line_kind(text: &str, comment: Option<&str>) -> Option<Kind> {
    if comment.is_some_and(|c| text.starts_with(c)) {
        Some(Kind::Comment)
    } else if IMPORTS.iter().any(|k| text.starts_with(k)) {
        Some(Kind::Import)
    } else {
        None
    }
}

/// Runs of lines all comments or all imports (each line's kind, in order), as regions
/// that fold to their first line: from it to the line after the run.
fn runs(kinds: impl Iterator<Item = Option<Kind>>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut run: Option<(usize, usize, Kind)> = None;
    let close = |run: Option<(usize, usize, Kind)>, out: &mut Vec<Range<usize>>| {
        if let Some((first, last, _)) = run
            && last + 1 - first >= RUN
        {
            out.push(first..last + 1);
        }
    };
    for (i, kind) in kinds.enumerate() {
        run = match (run, kind) {
            (Some((first, _, was)), Some(kind)) if was == kind => Some((first, i, kind)),
            (before, kind) => {
                close(before, &mut out);
                kind.map(|kind| (i, i, kind))
            }
        };
    }
    close(run, &mut out);
    out
}

/// How deep each of `regions` (sorted by start) is: 0 for one inside no other.
fn depths(regions: &[Range<usize>]) -> Vec<usize> {
    let mut open: Vec<Range<usize>> = Vec::new();
    regions
        .iter()
        .map(|r| {
            // Those it isn't inside any more (they ended before it, or don't hold it) close.
            while open.last().is_some_and(|o| !(o.start <= r.start && r.end <= o.end)) {
                open.pop();
            }
            let depth = open.len();
            open.push(r.clone());
            depth
        })
        .collect()
}

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
        let Some(highlighter) = &mut self.highlighter else {
            // Without a grammar: the indentation's blocks, innermost first.
            if self.basic_syntax().is_none() {
                return Vec::new();
            }
            let mut around: Vec<Range<usize>> =
                self.foldable().iter().filter(|b| b.start < line && line < b.end).cloned().collect();
            around.reverse();
            return around;
        };
        highlighter.sync(&self.buffer);
        highlighter.blocks_around(line)
    }

    /// The regions that can fold, worked out again only after edits.
    pub fn foldable(&mut self) -> &[Range<usize>] {
        let revision = self.buffer.revision();
        if self.folds.foldable.as_ref().is_none_or(|(r, _)| *r != revision) {
            // A language without a grammar (Swift, Ruby…): its blocks by indentation.
            let basic =
                self.basic_syntax().is_some() && self.buffer.rope().len_bytes() <= crate::basic_syntax::MAX_BYTES;
            let mut ranges = match &mut self.highlighter {
                Some(highlighter) => {
                    highlighter.sync(&self.buffer);
                    highlighter.fold_ranges()
                }
                None if basic => {
                    let lines: Vec<String> = (0..self.buffer.len_lines()).map(|l| self.buffer.line_text(l)).collect();
                    crate::basic_syntax::indent_blocks(lines.iter().map(String::as_str))
                }
                None => Vec::new(),
            };
            // In code: runs of comment lines, and of imports, fold too (not one the syntax
            // folds already). Only each line's start is looked at.
            if (self.highlighter.is_some() || basic) && !self.is_prose() {
                let comment = self.comment_marks().and_then(|(line, _)| line);
                // (Each line's start into one buffer, not a string per line.)
                let mut start = String::new();
                let mut kinds = Vec::with_capacity(self.buffer.len_lines());
                for line in self.buffer.rope().lines() {
                    start.clear();
                    start.extend(line.chars().skip_while(|c| *c == ' ' || *c == '\t').take(12));
                    kinds.push(line_kind(&start, comment));
                }
                for run in runs(kinds.into_iter()) {
                    if run.end < self.buffer.len_lines() && !ranges.iter().any(|r| r.start == run.start) {
                        ranges.push(run);
                    }
                }
                ranges.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
            }
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
        // Lines selected: those fold, behind the first.
        let range = self.selection.range();
        let (first, mut last) = (self.buffer.point(range.start).0, self.buffer.point(range.end));
        // (Whole lines selected end at the next one's start: that one isn't selected.)
        if last.1 == 0 && last.0 > first {
            last.0 -= 1;
        }
        let last = last.0;
        if last > first {
            return self.set_folded(first..last + 1, true, cx);
        }
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

    pub(super) fn fold_level_1(&mut self, _: &FoldLevel1, _: &mut Window, cx: &mut Context<Self>) {
        self.fold_level(1, cx);
    }

    pub(super) fn fold_level_2(&mut self, _: &FoldLevel2, _: &mut Window, cx: &mut Context<Self>) {
        self.fold_level(2, cx);
    }

    pub(super) fn fold_level_3(&mut self, _: &FoldLevel3, _: &mut Window, cx: &mut Context<Self>) {
        self.fold_level(3, cx);
    }

    /// Every block `level` deep folded (1: the outermost ones), every other one open: the
    /// file's outline at that depth.
    fn fold_level(&mut self, level: usize, cx: &mut Context<Self>) {
        let regions = self.foldable().to_vec();
        let depths = depths(&regions);
        self.folds.folded = regions.into_iter().zip(depths).filter(|(_, d)| d + 1 == level).map(|(r, _)| r).collect();
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
    fn runs_of_comments_and_imports_fold() {
        let kinds = |lines: &[&str]| lines.iter().map(|l| line_kind(l, Some("//"))).collect::<Vec<_>>();
        let lines = ["use a;", "use b;", "use c;", "", "// one", "// two", "fn f() {}", "// a", "// b", "// c", "x"];
        assert_eq!(runs(kinds(&lines).into_iter()), vec![0..3, 7..10]);
        // A run of two, or of two kinds: no.
        assert!(runs(kinds(&["// a", "use b;", "// c"]).into_iter()).is_empty());
    }

    #[test]
    fn nested_folds_hide_the_outer_lines_once() {
        assert_eq!(hidden_lines(&[0..10, 2..5, 12..14]), vec![1..10, 13..14]);
        assert_eq!(hidden_lines(&[3..4]), Vec::<Range<usize>>::new());
    }

    #[test]
    fn blocks_know_how_deep_they_are() {
        // impl { fn { if {} } fn {} } fn {}
        assert_eq!(depths(&[0..10, 1..5, 2..4, 6..9, 12..14]), [0, 1, 2, 1, 0]);
        // Ending on the same line as the one around it: still inside.
        assert_eq!(depths(&[0..5, 2..5]), [0, 1]);
        assert_eq!(depths(&[]), Vec::<usize>::new());
    }

    /// Fold Level 2: the methods inside an impl folded, the impl itself open.
    #[gpui::test]
    fn a_level_folds_its_blocks_only(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text =
            "impl A {\n    fn a() {\n        1;\n    }\n    fn b() {\n        2;\n    }\n}\nfn c() {\n    3;\n}\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some("a.rs".into()), cx));
        e.update_in(cx, |e, window, cx| {
            e.fold_level_2(&FoldLevel2, window, cx);
            assert_eq!(e.folds.folded, [1..3, 4..6]);
            e.fold_level_1(&FoldLevel1, window, cx);
            assert_eq!(e.folds.folded, [0..7, 8..10]);
        });
    }
}
