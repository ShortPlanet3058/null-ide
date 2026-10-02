//! Keyboard shortcuts: Null's own, or the ones from another editor you already know.
//!
//! Every part of the app registers its default keys; a preset then adds the keys
//! that editor uses differently. A later binding in the same context wins, and a
//! binding in a deeper context (the editor inside the workspace) wins over one
//! further out, so a preset only lists what changes.

use crate::editor::{
    AddCursorAbove, AddCursorBelow, AddNextOccurrence, DeleteLine, DuplicateLineDown, DuplicateLineUp, GoToDefinition,
    MoveLineDown, MoveLineUp, SelectAllOccurrences,
};
use crate::workspace::{GoToLine, ShowCommands, TogglePalette, ToggleSidebar, ToggleTerminal};
use gpui::{App, KeyBinding};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Keymap {
    #[default]
    Null,
    #[serde(rename = "vscode")]
    VsCode,
    JetBrains,
    Sublime,
    Zed,
}

impl Keymap {
    pub const ALL: [Keymap; 5] = [Keymap::Null, Keymap::VsCode, Keymap::JetBrains, Keymap::Sublime, Keymap::Zed];

    pub fn label(self) -> &'static str {
        match self {
            Keymap::Null => "Null",
            Keymap::VsCode => "VS Code",
            Keymap::JetBrains => "JetBrains",
            Keymap::Sublime => "Sublime Text",
            Keymap::Zed => "Zed",
        }
    }

    /// The keys that matter most, to recognise the preset by.
    pub fn summary(self) -> &'static str {
        match self {
            Keymap::Null => "⌘P files · ⌘K quick settings and commands · ⌘D next match · ⌥⇧↓ duplicate line",
            Keymap::VsCode => "⌘P files · ⇧⌘P commands · ⌘D next match · ⇧⌥↓ duplicate · ⇧⌘K delete line",
            Keymap::JetBrains => "⇧⌘O files · ⇧⌘A actions · ⌘L go to line · ⌘D duplicate · ⌘⌫ delete line",
            Keymap::Sublime => "⌘P files · ⇧⌘P commands · ⇧⌘D duplicate · ⌃⇧K delete line · ⌃⌘↑ move line",
            Keymap::Zed => "⌘P files · ⇧⌘P commands · ⌘D next match · ⇧⌘D duplicate · ⌃⇧K delete line",
        }
    }

    /// The keys this preset uses differently from Null's.
    fn overrides(self) -> Vec<KeyBinding> {
        let editor = Some("Editor");
        let workspace = Some("Workspace");
        match self {
            // Null's own keys already follow VS Code for nearly everything.
            Keymap::Null | Keymap::VsCode => Vec::new(),
            Keymap::JetBrains => vec![
                KeyBinding::new("secondary-shift-o", TogglePalette, workspace),
                KeyBinding::new("secondary-e", TogglePalette, workspace),
                KeyBinding::new("secondary-shift-a", ShowCommands, workspace),
                KeyBinding::new("secondary-1", ToggleSidebar, workspace),
                KeyBinding::new("alt-f12", ToggleTerminal, workspace),
                KeyBinding::new("secondary-l", GoToLine, editor),
                KeyBinding::new("secondary-d", DuplicateLineDown, editor),
                KeyBinding::new("secondary-backspace", DeleteLine, editor),
                KeyBinding::new("alt-shift-up", MoveLineUp, editor),
                KeyBinding::new("alt-shift-down", MoveLineDown, editor),
                KeyBinding::new("ctrl-g", AddNextOccurrence, editor),
                KeyBinding::new("ctrl-secondary-g", SelectAllOccurrences, editor),
                KeyBinding::new("secondary-b", GoToDefinition, editor),
            ],
            Keymap::Sublime => vec![
                KeyBinding::new("secondary-shift-d", DuplicateLineDown, editor),
                KeyBinding::new("ctrl-shift-k", DeleteLine, editor),
                KeyBinding::new("ctrl-secondary-up", MoveLineUp, editor),
                KeyBinding::new("ctrl-secondary-down", MoveLineDown, editor),
                KeyBinding::new("ctrl-secondary-g", SelectAllOccurrences, editor),
                KeyBinding::new("ctrl-shift-up", AddCursorAbove, editor),
                KeyBinding::new("ctrl-shift-down", AddCursorBelow, editor),
            ],
            Keymap::Zed => vec![
                KeyBinding::new("secondary-shift-d", DuplicateLineDown, editor),
                KeyBinding::new("ctrl-shift-k", DeleteLine, editor),
                KeyBinding::new("alt-shift-up", DuplicateLineUp, editor),
            ],
        }
    }
}

/// Registers every shortcut for `keymap`, replacing whatever was there.
pub fn register(keymap: Keymap, cx: &mut App) {
    cx.clear_key_bindings();
    crate::editor::bind_keys(cx);
    crate::workspace::bind_keys(cx);
    crate::file_tree::bind_keys(cx);
    crate::find_bar::bind_keys(cx);
    crate::terminal::bind_keys(cx);
    crate::key_prompt::bind_keys(cx);
    crate::text_input::bind_keys(cx);
    crate::settings_panel::bind_keys(cx);
    crate::welcome::bind_keys(cx);
    crate::editor::bind_ai_keys(cx);
    // After the text field's keys, so ←→ can change a choice in the palette.
    crate::palette::bind_keys(cx);
    cx.bind_keys(keymap.overrides());
    cx.bind_keys([KeyBinding::new("secondary-q", crate::menus::Quit, None)]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Action as _;

    #[test]
    fn presets_are_saved_by_short_names() {
        assert_eq!(serde_json::to_string(&Keymap::VsCode).unwrap(), "\"vscode\"");
        assert_eq!(serde_json::from_str::<Keymap>("\"jetbrains\"").unwrap(), Keymap::JetBrains);
    }

    /// The preset's keys win over Null's own: on JetBrains, ⌘D duplicates the line.
    #[gpui::test]
    fn a_preset_overrides_the_default_keys(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            register(Keymap::JetBrains, cx);
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            let stroke = gpui::Keystroke::parse("cmd-d").unwrap();
            let context = [gpui::KeyContext::parse("Workspace").unwrap(), gpui::KeyContext::parse("Editor").unwrap()];
            let (bindings, _) = keymap.bindings_for_input(&[stroke], &context);
            assert_eq!(bindings.first().map(|b| b.action().name()), Some(DuplicateLineDown.name()));
        });
    }
}
