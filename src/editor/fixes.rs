//! Quick fixes: ⌘. at a problem (or anywhere) lists what the language server can do
//! there, like adding a missing import or filling in match arms, in a small list under
//! the caret. The workspace applies the one picked, since it can touch other files.

use super::{Editor, EditorEvent};
use crate::fonts::Fonts;
use crate::theme::Theme;
use crate::ui;
use gpui::{
    AnyElement, App, ClickEvent, Context, KeyBinding, ScrollHandle, Window, actions, anchored, deferred, div, point,
    prelude::*, px,
};
use lsp_types::{CodeActionKind, CodeActionOrCommand};

actions!(fixes, [QuickFix]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-.", QuickFix, Some("Editor"))]);
}

pub struct FixMenu {
    pub fixes: Vec<CodeActionOrCommand>,
    pub selected: usize,
    /// Where the list hangs from.
    pub(super) at: usize,
    pub scroll: ScrollHandle,
    /// Set when the list holds a merge conflict's choices, resolved here, not by a server.
    pub conflict: Option<super::Conflict>,
    /// Set when the list holds a misspelled word's corrections (the word's chars).
    pub spelling: Option<std::ops::Range<usize>>,
}

/// The order fixes are listed in: the server's favourite, then fixes for the problem,
/// then everything else (refactorings, then whole-file actions), each as the server sent.
fn rank(fix: &CodeActionOrCommand) -> u8 {
    let CodeActionOrCommand::CodeAction(action) = fix else { return 2 };
    if action.is_preferred == Some(true) {
        return 0;
    }
    match &action.kind {
        Some(kind) if *kind == CodeActionKind::QUICKFIX => 1,
        Some(kind) if kind.as_str().starts_with("source") => 3,
        _ => 2,
    }
}

pub fn title(fix: &CodeActionOrCommand) -> &str {
    match fix {
        CodeActionOrCommand::Command(command) => &command.title,
        CodeActionOrCommand::CodeAction(action) => &action.title,
    }
}

/// Stands for "Fix with AI" in the list: not the server's, run here.
const AI_FIX: &str = "null.fixWithAi";

/// The list's last row when the caret's line has an error and AI is on: the error fixed
/// by AI, shown as a change to keep or take back, as ⌘I's are.
fn ai_fix() -> CodeActionOrCommand {
    CodeActionOrCommand::Command(lsp_types::Command {
        title: "Fix with AI".into(),
        command: AI_FIX.into(),
        arguments: None,
    })
}

fn is_ai_fix(fix: &CodeActionOrCommand) -> bool {
    matches!(fix, CodeActionOrCommand::Command(c) if c.command == AI_FIX)
}

/// The server's fixes, then AI's when it's offered.
fn with_ai_fix(mut fixes: Vec<CodeActionOrCommand>, offer: bool) -> Vec<CodeActionOrCommand> {
    if offer {
        fixes.push(ai_fix());
    }
    fixes
}

/// The fixes worth showing, best first. Ones the server says can't apply here are left out.
fn arrange(mut fixes: Vec<CodeActionOrCommand>) -> Vec<CodeActionOrCommand> {
    fixes.retain(|f| !matches!(f, CodeActionOrCommand::CodeAction(a) if a.disabled.is_some()));
    fixes.sort_by_key(rank);
    fixes
}

impl Editor {
    pub(super) fn quick_fix(&mut self, _: &QuickFix, _: &mut Window, cx: &mut Context<Self>) {
        let head = self.selection.head;
        if let Some(conflict) = self.conflict_at_caret() {
            return self.conflict_choices(conflict, cx);
        }
        // On a link to nothing: names nearby. On a misspelled word: its corrections.
        if self.broken_link_choices(cx) || self.spelling_choices(cx) {
            return;
        }
        if let Some(message) = self.not_ready_message(cx) {
            return self.show_notice(head, message, cx);
        }
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else {
            return self.show_notice(head, "Quick fixes come from a language server, and none runs here.".into(), cx);
        };
        let selection = self.selection.range();
        let mut range =
            lsp_types::Range { start: self.lsp_position(selection.start), end: self.lsp_position(selection.end) };
        // The problems on the lines around the caret, so the server offers their fixes.
        let lines = range.start.line..=range.end.line;
        let problems: Vec<_> = self
            .problems(cx)
            .iter()
            .filter(|p| {
                let (start, end) = (self.lsp_position(p.range.start).line, self.lsp_position(p.range.end).line);
                start <= *lines.end() && end >= *lines.start()
            })
            .cloned()
            .collect();
        // The caret only has to be on the problem's line, not on the problem itself.
        let inside = |p: &super::intel::Problem| p.range.start <= selection.start && selection.end <= p.range.end;
        if selection.is_empty()
            && !problems.iter().any(inside)
            && let Some(first) = problems.first()
        {
            range = lsp_types::Range {
                start: self.lsp_position(first.range.start),
                end: self.lsp_position(first.range.end),
            };
        }
        // An error here, and AI on: it can try too.
        let offer_ai = cx.global::<crate::settings::Settings>().ai.enabled
            && problems.iter().any(|p| p.severity == lsp_types::DiagnosticSeverity::ERROR);
        let diagnostics = problems.into_iter().map(|p| p.diagnostic).collect();
        let request = lsp.read(cx).code_actions(&path, range, diagnostics);
        let version = self.buffer.version();
        self.close_completion(cx);
        self.fixes_task = Some(cx.spawn(async move |this, cx| {
            let fixes = with_ai_fix(arrange(request.await), offer_ai);
            this.update(cx, |this, cx| {
                if this.buffer.version() != version {
                    return;
                }
                if fixes.is_empty() {
                    return this.show_notice(head, "No fixes here.".into(), cx);
                }
                this.close_hover(cx);
                this.fix_menu = Some(FixMenu {
                    fixes,
                    selected: 0,
                    at: head,
                    scroll: ScrollHandle::new(),
                    conflict: None,
                    spelling: None,
                });
                cx.notify();
            })
            .ok();
        }));
    }

    /// Whether the caret's line has a problem the server reported.
    pub fn caret_on_problem(&self, cx: &App) -> bool {
        let line = self.buffer.point(self.selection.head).0;
        self.problems(cx).iter().any(|p| {
            let (start, end) = (self.buffer.point(p.range.start).0, self.buffer.point(p.range.end).0);
            start <= line && line <= end
        })
    }

    pub(super) fn close_fixes(&mut self, cx: &mut Context<Self>) {
        self.fixes_task = None;
        if self.fix_menu.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn move_fix(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(menu) = &mut self.fix_menu else { return };
        let len = menu.fixes.len() as isize;
        menu.selected = (menu.selected as isize + delta).rem_euclid(len) as usize;
        menu.scroll.scroll_to_item(menu.selected);
        cx.notify();
    }

    /// Hands the fix to the workspace, which works out its edits and applies them.
    pub(super) fn accept_fix(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut menu) = self.fix_menu.take() else { return };
        if let Some(conflict) = menu.conflict {
            return self.resolve_conflict(conflict, ix, cx);
        }
        if let Some(word) = menu.spelling.clone() {
            if let Some(fix) = menu.fixes.get(ix) {
                self.accept_spelling(word, fix, cx);
            }
            return cx.notify();
        }
        if menu.fixes.get(ix).is_some_and(is_ai_fix) {
            return self.fix_with_ai(window, cx);
        }
        if ix < menu.fixes.len() {
            cx.emit(EditorEvent::CodeAction(menu.fixes.swap_remove(ix)));
        }
        cx.notify();
    }

    pub(super) fn render_fixes(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        const ROW: f32 = ui::ROW_SM;
        const ROWS: usize = 8;
        const PAD: f32 = 4.;
        const DOT_COLUMN: f32 = 18.;
        let menu = self.fix_menu.as_ref()?;
        let bounds = self.caret_bounds(menu.at)?;
        let theme = cx.global::<Theme>();
        let rows = menu.fixes.iter().enumerate().map(|(ix, fix)| {
            let selected = ix == menu.selected;
            // A dot for fixes to a problem; refactorings go without.
            let fixes_problem = rank(fix) <= 1;
            div()
                .id(ix)
                .h(px(ROW))
                .flex()
                .items_center()
                .pr(px(12.))
                .rounded(px(ui::R_ROW))
                .cursor_pointer()
                .when(selected, |row| row.bg(theme.accent_soft))
                .when(!selected, |row| row.hover(|r| r.bg(theme.hairline)))
                .child(
                    div()
                        .w(px(DOT_COLUMN))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .when(fixes_problem, |d| d.child(div().size(px(4.)).rounded_full().bg(theme.caret))),
                )
                .child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_color(if selected { theme.foreground } else { theme.muted })
                        .child(title(fix).to_string()),
                )
                .active(|s| s.opacity(0.7))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.accept_fix(ix, window, cx)))
        });
        let list = div()
            .id("fixes")
            .occlude()
            .track_scroll(&menu.scroll)
            .overflow_y_scroll()
            .min_w(px(220.))
            .max_w(px(560.))
            .max_h(px(ROW * ROWS as f32 + PAD * 2.))
            .p(px(PAD))
            .rounded(px(ui::R_POPOVER))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.hairline)
            .shadow_md()
            .font_family(cx.global::<Fonts>().ui.clone())
            .text_size(px(ui::T_MD))
            .children(rows);
        Some(
            deferred(
                anchored()
                    .position(point(bounds.left() - px(PAD + DOT_COLUMN), bounds.bottom() + px(3.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_comes_last_and_only_when_offered() {
        let server = vec![action("Import `HashMap`", Some(CodeActionKind::QUICKFIX), true)];
        let titles = |fixes: &[CodeActionOrCommand]| fixes.iter().map(|f| title(f).to_string()).collect::<Vec<_>>();
        assert_eq!(titles(&with_ai_fix(server.clone(), true)), ["Import `HashMap`", "Fix with AI"]);
        assert_eq!(titles(&with_ai_fix(server, false)), ["Import `HashMap`"]);
        assert!(is_ai_fix(&ai_fix()));
        assert!(!is_ai_fix(&action("Fix with AI", None, false)));
    }
    use lsp_types::{CodeAction, CodeActionDisabled, Command};

    fn action(title: &str, kind: Option<CodeActionKind>, preferred: bool) -> CodeActionOrCommand {
        CodeActionOrCommand::CodeAction(CodeAction {
            title: title.into(),
            kind,
            is_preferred: preferred.then_some(true),
            ..Default::default()
        })
    }

    #[test]
    fn preferred_fixes_come_first_and_disabled_ones_go() {
        let disabled = CodeActionOrCommand::CodeAction(CodeAction {
            title: "Inline".into(),
            disabled: Some(CodeActionDisabled { reason: "not here".into() }),
            ..Default::default()
        });
        let fixes = arrange(vec![
            action("Organize imports", Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS), false),
            action("Extract into function", Some(CodeActionKind::REFACTOR_EXTRACT), false),
            disabled,
            action("Import HashMap", Some(CodeActionKind::QUICKFIX), false),
            CodeActionOrCommand::Command(Command { title: "Run".into(), command: "run".into(), arguments: None }),
            action("Fill match arms", Some(CodeActionKind::QUICKFIX), true),
        ]);
        let titles: Vec<&str> = fixes.iter().map(title).collect();
        assert_eq!(titles, ["Fill match arms", "Import HashMap", "Extract into function", "Run", "Organize imports"]);
    }
}
