//! Gutter marks for lines changed since the last commit.

use super::Editor;
use crate::git::{self, Hunk};
use gpui::Context;
use std::sync::Arc;
use std::time::Duration;

/// Recompute after typing pauses this long.
const DIFF_PAUSE: Duration = Duration::from_millis(150);

impl Editor {
    /// Reads the committed version of the file again (it may have been committed since).
    pub fn reload_git_base(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return };
        self.git_base_task = Some(cx.spawn(async move |this, cx| {
            let base = cx.background_executor().spawn(async move { git::committed_text(&path) }).await;
            this.update(cx, |this, cx| {
                this.git_base = base.map(Arc::from);
                this.refresh_git_hunks(Duration::ZERO, cx);
            })
            .ok();
        }));
    }

    pub(super) fn text_changed_for_git(&mut self, cx: &mut Context<Self>) {
        self.refresh_git_hunks(DIFF_PAUSE, cx);
    }

    fn refresh_git_hunks(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let Some(base) = self.git_base.clone() else {
            if !self.git_hunks.is_empty() {
                self.git_hunks.clear();
                cx.notify();
            }
            return;
        };
        let current = self.buffer.to_string();
        self.git_diff_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let hunks: Vec<Hunk> = cx.background_executor().spawn(async move { git::diff(&base, &current) }).await;
            this.update(cx, |this, cx| {
                this.git_hunks = hunks;
                cx.notify();
            })
            .ok();
        }));
    }
}
