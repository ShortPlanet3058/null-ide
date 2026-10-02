//! Ghost completions: when you pause at the end of a line, the AI's guess at what
//! comes next shows faintly after the caret. ⇥ takes it, anything else ignores it.
//! Off unless switched on in Settings → AI.

use super::{EditKind, Editor};
use crate::ai::{self, AiEvent, Prompt};
use crate::settings::Settings;
use futures::StreamExt;
use gpui::{App, Context, KeyBinding, Window, actions};
use std::time::Duration;

actions!(ghost, [AcceptGhost, DismissGhost]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", AcceptGhost, Some("Editor && ai_ghost")),
        KeyBinding::new("escape", DismissGhost, Some("Editor && ai_ghost")),
    ]);
}

/// Wait this long after the last keystroke before asking.
const PAUSE: Duration = Duration::from_millis(450);
const MAX_LINES: usize = 8;
const PREFIX_LINES: usize = 80;
const SUFFIX_LINES: usize = 30;

pub(super) struct Ghost {
    /// Where it goes, and the text it was made for: it disappears once either changes.
    pub(super) offset: usize,
    pub(super) version: u64,
    pub(super) lines: Vec<String>,
}

impl Editor {
    /// The ghost's first line (drawn after the caret) and the lines below it.
    pub fn ghost_text(&self) -> Option<(&str, &[String])> {
        let ghost = self.ghost.as_ref()?;
        let (first, rest) = ghost.lines.split_first()?;
        Some((first.as_str(), rest))
    }

    pub fn ghost_line(&self) -> Option<usize> {
        self.ghost.as_ref().map(|g| self.buffer.point(g.offset).0)
    }

    /// Drops a ghost that no longer matches the caret or the text.
    pub(super) fn check_ghost(&mut self) {
        if self.ghost.as_ref().is_some_and(|g| g.offset != self.selection.head || g.version != self.buffer.version()) {
            self.ghost = None;
            self.rebuild_blocks();
        }
    }

    /// After typing: ask again once the typing pauses.
    pub(super) fn schedule_ghost(&mut self, cx: &mut Context<Self>) {
        self.ghost_task = None;
        let settings = cx.global::<Settings>();
        if !settings.ai.completions || settings.ai.active() == ai::ProviderId::Off {
            return;
        }
        if !self.selection.is_empty() || self.multi_cursor() || self.completion.is_some() || self.prompt.is_some() {
            return;
        }
        // Only at the end of a line (closing brackets after the caret are fine).
        let (line, col) = self.buffer.point(self.selection.head);
        let rest: String = self.buffer.line_text(line).chars().skip(col).collect();
        if !rest.chars().all(|c| c.is_whitespace() || matches!(c, ')' | ']' | '}' | ';' | ',' | '"' | '\'')) {
            return;
        }
        let offset = self.selection.head;
        let version = self.buffer.version();
        let first = line.saturating_sub(PREFIX_LINES);
        let prefix = self.buffer.slice(self.buffer.line_to_char(first)..offset);
        let last = (line + SUFFIX_LINES).min(self.buffer.len_lines());
        let suffix = self.buffer.slice(offset..self.buffer.line_to_char(last));
        let path = self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "untitled".into());
        let request = Prompt {
            system: format!(
                "You are a code completion engine in an editor. Continue the code exactly at <CURSOR>. Reply \
                 with only the text to insert there: no explanation, no markdown fences, no repeating what's \
                 before the cursor. At most {MAX_LINES} lines; stop at a natural end, like the end of a statement \
                 or a block. If nothing obvious comes next, reply with nothing."
            ),
            user: format!("File: {path} ({})\n\n{prefix}<CURSOR>{suffix}", self.language_name()),
        };
        let ai_settings = settings.ai.clone();
        self.ghost_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE).await;
            let still_there = this
                .update(cx, |this, _| this.selection.head == offset && this.buffer.version() == version)
                .unwrap_or(false);
            if !still_there {
                return;
            }
            let mut events = ai::stream(ai_settings, request);
            let mut text = String::new();
            while let Some(event) = events.next().await {
                match event {
                    AiEvent::Text(chunk) => text.push_str(&chunk),
                    AiEvent::Done => break,
                    AiEvent::Failed(_) => return,
                }
            }
            this.update(cx, |this, cx| {
                if this.selection.head != offset || this.buffer.version() != version {
                    return;
                }
                let text = ai::strip_code_fence(&text);
                let lines: Vec<String> = text.trim_end().lines().take(MAX_LINES).map(str::to_string).collect();
                if lines.iter().all(|l| l.trim().is_empty()) {
                    return;
                }
                this.ghost = Some(Ghost { offset, version, lines });
                this.rebuild_blocks();
                cx.notify();
            })
            .ok();
        }));
    }

    fn accept_ghost(&mut self, _: &AcceptGhost, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ghost) = self.ghost.take() else { return };
        let text = ghost.lines.join("\n");
        self.edit(ghost.offset..ghost.offset, &text, EditKind::Other, cx);
        self.rebuild_blocks();
    }

    fn dismiss_ghost(&mut self, _: &DismissGhost, _: &mut Window, cx: &mut Context<Self>) {
        self.ghost = None;
        self.ghost_task = None;
        self.rebuild_blocks();
        cx.notify();
    }

    pub(super) fn accept_ghost_action(&mut self, a: &AcceptGhost, w: &mut Window, cx: &mut Context<Self>) {
        self.accept_ghost(a, w, cx)
    }

    pub(super) fn dismiss_ghost_action(&mut self, a: &DismissGhost, w: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_ghost(a, w, cx)
    }
}
