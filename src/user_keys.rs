//! Your own shortcuts, from `"keys"` in settings.json, over the preset's:
//!
//! ```json
//! "keys": { "ctrl-cmd-l": "select all occurrences", "cmd-d": null }
//! ```
//!
//! A command is named as Null calls it in the code (`editor::SelectAllOccurrences`), without
//! the part before `::`, or in words, as ⌘K lists it; `null` takes a shortcut away. They hold
//! everywhere in the window (bound with no context, they match as deep as anything focused)
//! and come last, so they win. A key that types something (`x`, `⇧X`, a space) can't be one:
//! it would stop typing.

use gpui::{Action, App, Global, KeyBinding, Keystroke, NoAction};
use std::collections::BTreeMap;

/// What was wrong with the shortcuts last read, for the person to see.
#[derive(Default)]
pub struct KeyProblems(pub Vec<String>);

impl Global for KeyProblems {}

/// Whether `keystrokes` start with a key that types a character: a letter, a digit, a sign
/// or a space, with no modifier or only ⇧.
fn types_something(keystrokes: &str) -> bool {
    let Some(first) = keystrokes.split_whitespace().next().and_then(|k| Keystroke::parse(k).ok()) else {
        return false;
    };
    let m = first.modifiers;
    let plain = !(m.platform || m.control || m.alt || m.function);
    plain && (first.key.chars().count() == 1 || first.key == "space")
}

/// A name as written, for comparing: letters and digits, in lower case.
fn plain(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// The command `name` stands for, among `known` (the full names, `editor::Foo`).
fn command_named<'a>(name: &str, known: &[&'a str]) -> Option<&'a str> {
    if let Some(exact) = known.iter().find(|k| **k == name) {
        return Some(exact);
    }
    let wanted = plain(name.rsplit("::").next().unwrap_or(name));
    if wanted.is_empty() {
        return None;
    }
    // Several with that name (`editor::Copy`, `terminal::Copy`): the editor's.
    let matching: Vec<&'a str> =
        known.iter().copied().filter(|k| plain(k.rsplit("::").next().unwrap_or(k)) == wanted).collect();
    matching.iter().find(|k| k.starts_with("editor::")).or(matching.first()).copied()
}

/// The bindings for `keys`, and what couldn't be understood.
pub fn bindings(keys: &BTreeMap<String, Option<String>>, cx: &App) -> (Vec<KeyBinding>, Vec<String>) {
    let known = cx.all_action_names();
    let mut bindings = Vec::new();
    let mut problems = Vec::new();
    for (keystrokes, name) in keys {
        let action: Box<dyn Action> = match name {
            None => Box::new(NoAction),
            Some(name) => match command_named(name, known).and_then(|full| cx.build_action(full, None).ok()) {
                Some(action) => action,
                None => {
                    problems.push(format!("\"{keystrokes}\": no command called \"{name}\""));
                    continue;
                }
            },
        };
        if types_something(keystrokes) {
            problems.push(format!("\"{keystrokes}\" types a character: add cmd, ctrl or alt"));
            continue;
        }
        match KeyBinding::load(keystrokes, action, None, false, None, &gpui::DummyKeyboardMapper) {
            Ok(binding) => bindings.push(binding),
            Err(_) => problems.push(format!("\"{keystrokes}\" isn't a shortcut (like \"cmd-shift-l\")")),
        }
    }
    (bindings, problems)
}

/// Binds your shortcuts, after every other key, and keeps what was wrong with them.
pub fn register(cx: &mut App) {
    let keys = cx.try_global::<crate::settings::Settings>().map(|s| s.keys.clone()).unwrap_or_default();
    let (bindings, problems) = bindings(&keys, cx);
    cx.bind_keys(bindings);
    cx.set_global(KeyProblems(problems));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_that_type_are_refused() {
        assert!(types_something("x") && types_something("shift-x") && types_something("space"));
        assert!(types_something("5") && types_something("shift-/"));
        assert!(!types_something("cmd-x") && !types_something("ctrl-x") && !types_something("alt-x"));
        assert!(!types_something("f5") && !types_something("escape") && !types_something("up"));
    }

    #[test]
    fn commands_are_found_however_they_re_named() {
        let known = ["editor::SelectAllOccurrences", "editor::Copy", "terminal::Copy", "workspace::ToggleTerminal"];
        assert_eq!(command_named("editor::SelectAllOccurrences", &known), Some("editor::SelectAllOccurrences"));
        assert_eq!(command_named("SelectAllOccurrences", &known), Some("editor::SelectAllOccurrences"));
        assert_eq!(command_named("select all occurrences", &known), Some("editor::SelectAllOccurrences"));
        assert_eq!(command_named("Toggle Terminal", &known), Some("workspace::ToggleTerminal"));
        assert_eq!(command_named("copy", &known), Some("editor::Copy"), "the editor's, of several");
        assert_eq!(command_named("terminal::Copy", &known), Some("terminal::Copy"));
        assert_eq!(command_named("launch rockets", &known), None);
        assert_eq!(command_named("::", &known), None);
    }

    /// Your keys win over Null's, `null` takes one away, and mistakes are kept to be shown.
    #[gpui::test]
    fn your_own_shortcuts_win(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        use crate::editor::{Editor, Selection};
        cx.update(|cx| {
            let keys = BTreeMap::from([
                ("ctrl-cmd-l".to_string(), Some("select all occurrences".to_string())),
                ("cmd-d".to_string(), None),
                ("cmd-alt-9".to_string(), Some("launch rockets".to_string())),
                ("cmd-shiftt-x".to_string(), Some("Copy".to_string())),
                ("x".to_string(), Some("toggle terminal".to_string())),
            ]);
            cx.set_global(crate::settings::Settings { keys, ..Default::default() });
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let problems = cx.update(|cx| cx.global::<KeyProblems>().0.clone());
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("cmd-alt-9") && problems[0].contains("launch rockets"));
        assert!(problems[1].contains("cmd-shiftt-x"));
        assert!(problems[2].contains("types a character"), "a plain letter would stop typing");
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text("a b a b a\n"), Some("x.txt".into()), cx));
        e.update_in(cx, |e, window, cx| {
            window.focus(&gpui::Focusable::focus_handle(e, cx));
            e.selection = Selection { anchor: 0, head: 1 };
        });
        // ⌘D, taken away: still one selection.
        cx.simulate_keystrokes("cmd-d");
        assert_eq!(e.read_with(cx, |e, _| e.extra.len()), 0);
        // ⌃⌘L, yours: every `a`.
        cx.simulate_keystrokes("ctrl-cmd-l");
        assert_eq!(e.read_with(cx, |e, _| e.extra.len()), 2);
    }
}
