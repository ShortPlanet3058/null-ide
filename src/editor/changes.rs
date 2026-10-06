//! Gutter marks for lines changed since the last commit, and who last changed the
//! caret's line.

use super::Editor;
use crate::git::{self, Hunk};
use gpui::Context;
use std::sync::Arc;
use std::time::Duration;

/// Recompute after typing pauses this long.
const DIFF_PAUSE: Duration = Duration::from_millis(150);
/// How wide the strip of the gutter is where a click opens a change (pixels).
const MARK_STRIP: f32 = 12.;
/// The caret rests on a line this long before saying who changed it.
const BLAME_PAUSE: Duration = Duration::from_millis(700);

/// Who last changed a line, as shown at its end.
pub(super) struct Blame {
    line: usize,
    revision: u64,
    text: String,
}

impl Editor {
    /// Reads the committed version of the file again (it may have been committed since).
    pub fn reload_git_base(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return };
        let encoding = self.encoding;
        self.git_base_task = Some(cx.spawn(async move |this, cx| {
            let base = cx.background_executor().spawn(async move { git::committed_text(&path, encoding) }).await;
            this.update(cx, |this, cx| {
                this.git_base = base.map(Arc::from);
                this.refresh_git_hunks(Duration::ZERO, cx);
            })
            .ok();
        }));
    }

    /// The changed line whose gutter mark is at `position`, if one is: the mark sits in the
    /// last strip of the gutter, just before the text.
    pub(super) fn change_mark_at(&self, position: gpui::Point<gpui::Pixels>) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let strip = layout.text_bounds.left() - gpui::px(MARK_STRIP)..layout.text_bounds.left();
        if !layout.bounds.contains(&position) || !strip.contains(&position.x) {
            return None;
        }
        let row = ((position.y - layout.text_origin.y) / layout.line_height).floor();
        let r = layout.rows.get((row as usize).checked_sub(layout.first_row)?)?;
        let line = r.row.line;
        self.git_hunks
            .iter()
            .any(|h| match h.change {
                git::Change::Deleted => h.lines.start == line,
                _ => h.lines.contains(&line),
            })
            .then_some(line)
    }

    /// A change's mark clicked: the file's changes since the last commit to keep or take
    /// back, the caret in that one (⇥ keeps it, Esc takes it back).
    pub(super) fn review_change_at(&mut self, line: usize, cx: &mut Context<Self>) {
        let Some(base) = self.git_base.clone() else { return };
        self.start_review(base.to_string(), cx);
        self.single_cursor();
        self.selection = super::Selection::caret(self.buffer.line_to_char(line));
        self.touch(cx);
    }

    /// "You, 3 days ago · Fix the parser" for the caret's line, while it's still true.
    pub fn line_blame(&self) -> Option<(usize, &str)> {
        let blame = self.blame.as_ref().filter(|b| b.revision == self.buffer.revision())?;
        Some((blame.line, blame.text.as_str()))
    }

    /// After the caret moves: keep what's shown for its line, or ask git once it rests.
    pub(super) fn refresh_blame(&mut self, cx: &mut Context<Self>) {
        let line = self.buffer.point(self.selection.head).0;
        let revision = self.buffer.revision();
        if self.blame.as_ref().is_some_and(|b| b.line == line && b.revision == revision) {
            return;
        }
        self.blame = None;
        self.blame_task = None;
        let wanted = cx.global::<crate::settings::Settings>().line_blame
            && self.selection.is_empty()
            && self.extra.is_empty()
            && !self.in_review();
        // Only for files git follows (they have a committed version).
        let (Some(path), true, true) = (self.path.clone(), self.git_base.is_some(), wanted) else { return };
        let text = self.buffer.rope().clone();
        self.blame_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BLAME_PAUSE).await;
            let blame =
                cx.background_executor().spawn(async move { git::blame_line(&path, &text.to_string(), line) }).await;
            let Some(blame) = blame else { return };
            let now =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            let mut text = format!("{}, {}", blame.author, git::ago(blame.time, now));
            if !blame.summary.is_empty() {
                // A long commit title is cut short: it's a hint, not the log.
                const LONGEST: usize = 60;
                text.push_str(" · ");
                if blame.summary.chars().count() > LONGEST {
                    text.extend(blame.summary.chars().take(LONGEST - 1));
                    text.push('…');
                } else {
                    text.push_str(&blame.summary);
                }
            }
            this.update(cx, |this, cx| {
                if this.buffer.revision() == revision && this.buffer.point(this.selection.head).0 == line {
                    this.blame = Some(Blame { line, revision, text });
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    pub(super) fn text_changed_for_git(&mut self, cx: &mut Context<Self>) {
        self.refresh_git_hunks(DIFF_PAUSE, cx);
    }

    pub(super) fn refresh_git_hunks(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let Some(base) = self.git_base.clone() else {
            if !self.git_hunks.is_empty() {
                self.git_hunks.clear();
                cx.notify();
            }
            return;
        };
        // A rope copy is a few pointers; the text itself is only built after the pause, off this thread.
        let current = self.buffer.rope().clone();
        self.git_diff_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let hunks: Vec<Hunk> =
                cx.background_executor().spawn(async move { git::diff(&base, &current.to_string()) }).await;
            this.update(cx, |this, cx| {
                this.git_hunks = hunks;
                cx.notify();
            })
            .ok();
        }));
    }
}
