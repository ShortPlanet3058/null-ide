//! Code lens: what the language server says over a function ("3 references", "▶ Run
//! Test"), at the end of the caret's line only, faintly, and each part clickable.
//! Asked for once typing pauses; a lens the server gives without its words is asked for
//! them only when the caret comes to its line.

use super::{Editor, EditorEvent, Selection};
use crate::lsp_store::Readiness;
use gpui::{Bounds, Context, Pixels, Task, Window};
use std::time::Duration;

/// Ask again once typing has paused this long.
const PAUSE: Duration = Duration::from_millis(600);
/// While the server starts or reads the project, look again this often.
const NOT_READY: Duration = Duration::from_secs(1);

/// What clicking a lens does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LensAction {
    /// Where it's used, listed (at a char offset).
    References(usize),
    /// Its implementations.
    Implementations(usize),
    /// A command, run in the terminal (`cargo test -- --exact x`).
    Run(String),
}

/// One part of a line's lens: its words, and what a click does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LensItem {
    pub title: String,
    pub action: LensAction,
}

#[derive(Default)]
pub(super) struct Lenses {
    /// What the server gave, each with its line; in order.
    raw: Vec<(usize, lsp_types::CodeLens)>,
    /// The revision `raw` is for (shown only while the text is as it was).
    revision: u64,
    answered: Option<u64>,
    task: Option<Task<()>>,
    /// Lines whose lenses were asked for their words, and those given (for `revision`).
    resolving: Option<(usize, Task<()>)>,
    /// Where each lens part is drawn, set while drawing, for clicks.
    pub hits: Vec<(Bounds<Pixels>, LensAction)>,
}

/// Arguments joined as a shell line, each quoted when it needs to be.
fn shell_line(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            let plain =
                !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | '@' | '+'));
            if plain { a.clone() } else { format!("'{}'", a.replace('\'', r"'\''")) }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a lens's command does here: rust-analyzer's run and references commands, and any
/// server's "N references" or "N implementations". None for one Null can't do.
pub fn action_of(lens: &lsp_types::CodeLens, offset: usize) -> Option<LensItem> {
    let command = lens.command.as_ref()?;
    let title = command.title.trim().to_string();
    let lower = title.to_lowercase();
    let action = match command.command.as_str() {
        "rust-analyzer.runSingle" => {
            let args = command.arguments.as_ref()?.first()?.get("args")?;
            let strings = |key: &str| -> Vec<String> {
                args.get(key)
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                    .unwrap_or_default()
            };
            let cargo = strings("cargoArgs");
            if cargo.is_empty() {
                return None;
            }
            let mut line = format!("cargo {}", shell_line(&cargo));
            let after = strings("executableArgs");
            if !after.is_empty() {
                line.push_str(&format!(" -- {}", shell_line(&after)));
            }
            LensAction::Run(line)
        }
        _ if lower.contains("implementation") => LensAction::Implementations(offset),
        _ if lower.contains("reference") || lower.contains("usage") => LensAction::References(offset),
        _ => return None,
    };
    Some(LensItem { title, action })
}

impl Editor {
    /// While lenses are on: asks the server for this text's, once typing pauses.
    pub fn ensure_lenses(&mut self, cx: &mut Context<Self>) {
        let revision = self.buffer.revision();
        if self.lenses.task.is_some() || self.lenses.answered == Some(revision) {
            return;
        }
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else { return };
        if !lsp.read(cx).has_server_for(&path) {
            return;
        }
        self.lenses.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE).await;
            loop {
                let Ok(readiness) = this.update(cx, |this, cx| this.readiness(cx)) else { return };
                match readiness {
                    Some(Readiness::Ready { .. }) => break,
                    Some(Readiness::Starting | Readiness::Indexing { .. } | Readiness::Installing { .. }) => {
                        cx.background_executor().timer(NOT_READY).await;
                    }
                    _ => {
                        this.update(cx, |this, _| this.lenses.answered = Some(revision)).ok();
                        return;
                    }
                }
            }
            let Ok(request) = this.update(cx, |_, cx| lsp.read(cx).code_lenses(&path)) else { return };
            let found = request.await;
            this.update(cx, |this, cx| {
                this.lenses.task = None;
                if this.buffer.revision() != revision {
                    return;
                }
                let mut raw: Vec<(usize, lsp_types::CodeLens)> =
                    found.into_iter().map(|l| (l.range.start.line as usize, l)).collect();
                raw.sort_by_key(|(line, _)| *line);
                this.lenses = Lenses { raw, revision, answered: Some(revision), ..Lenses::default() };
                cx.notify();
            })
            .ok();
        }));
    }

    /// The lens of `line` (the caret's), its words asked for if the server left them out:
    /// nothing while the text has changed since it was given.
    pub fn lens_on_line(&mut self, line: usize, cx: &mut Context<Self>) -> Vec<LensItem> {
        if self.lenses.revision != self.buffer.revision() {
            return Vec::new();
        }
        let on_line: Vec<lsp_types::CodeLens> =
            self.lenses.raw.iter().filter(|(l, _)| *l == line).map(|(_, lens)| lens.clone()).collect();
        if on_line.iter().any(|l| l.command.is_none()) {
            self.resolve_lenses(line, on_line, cx);
            return Vec::new();
        }
        on_line.iter().filter_map(|lens| action_of(lens, self.offset_from_lsp(lens.range.start))).collect()
    }

    fn resolve_lenses(&mut self, line: usize, lenses: Vec<lsp_types::CodeLens>, cx: &mut Context<Self>) {
        if self.lenses.resolving.as_ref().is_some_and(|(l, _)| *l == line) {
            return;
        }
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else { return };
        let revision = self.lenses.revision;
        let task = cx.spawn(async move |this, cx| {
            let mut resolved = Vec::new();
            for lens in lenses {
                if lens.command.is_some() {
                    resolved.push(lens);
                    continue;
                }
                let Ok(request) = this.update(cx, |_, cx| lsp.read(cx).resolve_code_lens(&path, lens.clone())) else {
                    return;
                };
                // One the server can't put words to: shown without it.
                resolved.push(request.await.unwrap_or(lens));
            }
            this.update(cx, |this, cx| {
                this.lenses.resolving = None;
                if this.lenses.revision != revision {
                    return;
                }
                // Words or not, it's been asked: a lens left without them isn't asked again.
                let mut resolved = resolved.into_iter();
                for (l, lens) in this.lenses.raw.iter_mut() {
                    if *l == line
                        && let Some(new) = resolved.next()
                    {
                        *lens = new;
                        if lens.command.is_none() {
                            lens.command = Some(lsp_types::Command::new(String::new(), String::new(), None));
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.lenses.resolving = Some((line, task));
    }

    /// Where the lens parts were drawn this frame (for clicks).
    pub fn set_lens_hits(&mut self, hits: Vec<(Bounds<Pixels>, LensAction)>) {
        self.lenses.hits = hits;
    }

    /// A click on a lens part: what it does, done. True when it was one.
    pub(super) fn click_lens(&mut self, position: gpui::Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(action) = self.lenses.hits.iter().find(|(b, _)| b.contains(&position)).map(|(_, a)| a.clone()) else {
            return false;
        };
        match action {
            LensAction::References(offset) => {
                self.selection = Selection::caret(offset);
                self.find_references_at(offset, cx);
            }
            LensAction::Implementations(offset) => {
                self.selection = Selection::caret(offset);
                self.go_to_target(crate::lsp_store::Target::Implementation, cx);
            }
            LensAction::Run(command) => cx.emit(EditorEvent::RunCommand(command)),
        }
        let _ = window;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens(title: &str, command: &str, arguments: Option<Vec<serde_json::Value>>) -> lsp_types::CodeLens {
        lsp_types::CodeLens {
            range: lsp_types::Range::default(),
            command: Some(lsp_types::Command::new(title.into(), command.into(), arguments)),
            data: None,
        }
    }

    /// The caret's line shows its lenses; a click on one does what it says.
    #[gpui::test]
    fn a_lens_is_clicked(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(Buffer::from_text("fn adds() {}\nfn other() {}\n"), Some("a.rs".into()), cx)
        });
        let ran = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let seen = ran.clone();
        cx.update(|_, cx| {
            cx.subscribe(&e, move |_, event: &EditorEvent, _| {
                if let EditorEvent::RunCommand(command) = event {
                    seen.borrow_mut().push(command.clone());
                }
            })
            .detach()
        });
        e.update_in(cx, |e, window, cx| {
            let mut run = lens("▶ Run Test", "rust-analyzer.runSingle", Some(vec![serde_json::json!({
                "args": { "cargoArgs": ["test", "--", "adds", "--exact"] }
            })]));
            run.range = lsp_types::Range::new(lsp_types::Position::new(0, 3), lsp_types::Position::new(0, 7));
            e.lenses = Lenses { raw: vec![(0, run)], revision: e.buffer.revision(), ..Lenses::default() };
            let items = e.lens_on_line(0, cx);
            assert_eq!(items.len(), 1);
            assert!(e.lens_on_line(1, cx).is_empty(), "other lines: nothing");
            let at = gpui::Bounds::new(gpui::point(gpui::px(100.), gpui::px(0.)), gpui::size(gpui::px(60.), gpui::px(20.)));
            e.set_lens_hits(vec![(at, items[0].action.clone())]);
            assert!(e.click_lens(gpui::point(gpui::px(120.), gpui::px(10.)), window, cx));
            assert!(!e.click_lens(gpui::point(gpui::px(10.), gpui::px(10.)), window, cx), "not on it");
        });
        cx.run_until_parked();
        assert_eq!(*ran.borrow(), ["cargo test -- adds --exact"]);
    }

    #[test]
    fn lenses_say_what_a_click_does() {
        let run = lens(
            "▶\u{fe0e} Run Test",
            "rust-analyzer.runSingle",
            Some(vec![serde_json::json!({
                "label": "test tests::adds",
                "kind": "cargo",
                "args": {
                    "cargoArgs": ["test", "--package", "shop", "--lib", "--", "tests::adds", "--exact"],
                    "executableArgs": ["--nocapture"]
                }
            })]),
        );
        assert_eq!(
            action_of(&run, 0).unwrap().action,
            LensAction::Run("cargo test --package shop --lib -- tests::adds --exact -- --nocapture".into())
        );
        let refs = lens("3 references", "rust-analyzer.showReferences", None);
        assert_eq!(action_of(&refs, 7).unwrap().action, LensAction::References(7));
        let imps = lens("2 implementations", "rust-analyzer.showReferences", None);
        assert_eq!(action_of(&imps, 7).unwrap().action, LensAction::Implementations(7));
        assert_eq!(action_of(&lens("Debug", "rust-analyzer.debugSingle", None), 0), None, "not one Null can do");
        let spaced = shell_line(&["test".into(), "it's".into()]);
        assert_eq!(spaced, r"test 'it'\''s'");
    }
}
