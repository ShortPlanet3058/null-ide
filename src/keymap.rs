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
    /// Vim's modes and keys in the editor, Null's ⌘ keys everywhere.
    Vim,
}

impl Keymap {
    pub const ALL: [Keymap; 6] =
        [Keymap::Null, Keymap::VsCode, Keymap::JetBrains, Keymap::Sublime, Keymap::Zed, Keymap::Vim];

    pub fn label(self) -> &'static str {
        match self {
            Keymap::Null => "Null",
            Keymap::VsCode => "VS Code",
            Keymap::JetBrains => "JetBrains",
            Keymap::Sublime => "Sublime Text",
            Keymap::Zed => "Zed",
            Keymap::Vim => "Vim",
        }
    }

    /// The keys that matter most, to recognise the preset by.
    pub fn summary(self) -> &'static str {
        match self {
            Keymap::Null => mac_or(
                "⌘P files · ⌘K quick settings and commands · ⌘D next match · ⌥⇧↓ duplicate line",
                "Ctrl+P files · Ctrl+K quick settings and commands · Ctrl+D next match · Alt+Shift+↓ duplicate line",
            ),
            Keymap::VsCode => mac_or(
                "⌘P files · ⇧⌘P commands · ⌘D next match · ⌥⇧↓ duplicate · ⇧⌘K delete line",
                "Ctrl+P files · Ctrl+Shift+P commands · Ctrl+D next match · Alt+Shift+↓ duplicate · Ctrl+Shift+K delete line",
            ),
            Keymap::JetBrains => mac_or(
                "⇧⌘O files · ⇧⌘A actions · ⌘L go to line · ⌘D duplicate · ⌘⌫ delete line",
                "Ctrl+Shift+O files · Ctrl+Shift+A actions · Ctrl+G go to line · Ctrl+D duplicate · Ctrl+Y delete line",
            ),
            Keymap::Sublime => mac_or(
                "⌘P files · ⇧⌘P commands · ⇧⌘D duplicate · ⌃⇧K delete line · ⌃⌘↑ move line",
                "Ctrl+P files · Ctrl+Shift+P commands · Ctrl+Shift+D duplicate · Ctrl+Shift+K delete line · Ctrl+Shift+↑ move line",
            ),
            Keymap::Zed => mac_or(
                "⌘P files · ⇧⌘P commands · ⌘D next match · ⇧⌘D duplicate · ⌃⇧K delete line",
                "Ctrl+P files · Ctrl+Shift+P commands · Ctrl+D next match · Ctrl+Shift+D duplicate · Ctrl+Shift+K delete line",
            ),
            Keymap::Vim => mac_or(
                "hjkl w b e move · i a o insert · v V select · d c y p · Esc · ⌘ keys as Null's",
                "hjkl w b e move · i a o insert · v V select · d c y p · Esc · other Ctrl keys as Null's",
            ),
        }
    }

    /// The keys this preset uses differently from Null's.
    fn overrides(self) -> Vec<KeyBinding> {
        let editor = Some("Editor");
        let workspace = Some("Workspace");
        match self {
            // Null's own keys already follow VS Code for nearly everything.
            Keymap::Null | Keymap::VsCode | Keymap::Vim => Vec::new(),
            Keymap::JetBrains => vec![
                KeyBinding::new("secondary-shift-o", TogglePalette, workspace),
                KeyBinding::new("secondary-e", TogglePalette, workspace),
                // Over the editor's own ⌘E (use the selection for find).
                KeyBinding::new("secondary-e", TogglePalette, editor),
                KeyBinding::new("secondary-shift-a", ShowCommands, workspace),
                KeyBinding::new("secondary-1", ToggleSidebar, workspace),
                KeyBinding::new("alt-f12", ToggleTerminal, workspace),
                // (Over the editor's own ⌥F12 and ⌘F12: Peek Definition, Go to Implementation.)
                KeyBinding::new("alt-f12", ToggleTerminal, editor),
                KeyBinding::new(mac_or("cmd-l", "ctrl-g"), GoToLine, editor),
                KeyBinding::new("secondary-d", DuplicateLineDown, editor),
                // (Off the Mac, Ctrl+Backspace deletes a word: JetBrains' Ctrl+Y there.)
                KeyBinding::new(mac_or("cmd-backspace", "ctrl-y"), DeleteLine, editor),
                KeyBinding::new("alt-shift-up", MoveLineUp, editor),
                KeyBinding::new("alt-shift-down", MoveLineDown, editor),
                KeyBinding::new(mac_or("ctrl-g", "alt-j"), AddNextOccurrence, editor),
                KeyBinding::new(mac_or("ctrl-cmd-g", "ctrl-alt-shift-j"), SelectAllOccurrences, editor),
                KeyBinding::new("secondary-b", GoToDefinition, editor),
                KeyBinding::new("secondary-f12", crate::workspace::GoToSymbol, workspace),
                KeyBinding::new("secondary-f12", crate::workspace::GoToSymbol, editor),
                KeyBinding::new("secondary-alt-o", crate::workspace::GoToSymbolInProject, workspace),
                KeyBinding::new("ctrl-shift-j", crate::editor::JoinLines, editor),
            ]
            .into_iter()
            .chain(jetbrains_platform_keys())
            .collect(),
            Keymap::Sublime => vec![
                // ⌘R is rename elsewhere; in Sublime it lists the file's symbols.
                KeyBinding::new("secondary-r", crate::workspace::GoToSymbol, editor),
                KeyBinding::new("secondary-r", crate::workspace::GoToSymbol, workspace),
                KeyBinding::new("secondary-shift-r", crate::workspace::GoToSymbolInProject, workspace),
                KeyBinding::new("secondary-shift-d", crate::editor::DuplicateSelection, editor),
                KeyBinding::new("ctrl-shift-k", DeleteLine, editor),
                // (Sublime's own keys on Windows and Linux, off the Mac.)
                KeyBinding::new(mac_or("ctrl-cmd-up", "ctrl-shift-up"), MoveLineUp, editor),
                KeyBinding::new(mac_or("ctrl-cmd-down", "ctrl-shift-down"), MoveLineDown, editor),
                KeyBinding::new(mac_or("ctrl-cmd-g", "alt-f3"), SelectAllOccurrences, editor),
                KeyBinding::new(mac_or("ctrl-shift-up", "ctrl-alt-up"), AddCursorAbove, editor),
                KeyBinding::new(mac_or("ctrl-shift-down", "ctrl-alt-down"), AddCursorBelow, editor),
                // Sublime goes round its bookmarks with F2 (renaming has no key there).
                KeyBinding::new("f2", crate::editor::NextBookmark, editor),
                KeyBinding::new("shift-f2", crate::editor::PreviousBookmark, editor),
            ],
            Keymap::Zed => vec![
                KeyBinding::new("secondary-shift-d", DuplicateLineDown, editor),
                KeyBinding::new("ctrl-shift-k", DeleteLine, editor),
                KeyBinding::new("alt-shift-up", DuplicateLineUp, editor),
            ],
        }
    }
}

/// JetBrains keys that differ between macOS and the others.
/// `mac` on the Mac, `other` elsewhere: a preset's keys as that editor has them on each.
fn mac_or(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") { mac } else { other }
}

fn jetbrains_platform_keys() -> Vec<KeyBinding> {
    use crate::editor::{ExpandSelection, ShrinkSelection};
    use crate::workspace::{GoBack, GoForward};
    let editor = Some("Editor");
    let workspace = Some("Workspace");
    if cfg!(target_os = "macos") {
        vec![
            // Over the editor's own ⌘[ and ⌘] (outdent, indent), as in JetBrains.
            KeyBinding::new("cmd-[", GoBack, editor),
            KeyBinding::new("cmd-]", GoForward, editor),
            KeyBinding::new("cmd-[", GoBack, workspace),
            KeyBinding::new("cmd-]", GoForward, workspace),
            KeyBinding::new("alt-up", ExpandSelection, editor),
            KeyBinding::new("alt-down", ShrinkSelection, editor),
        ]
    } else {
        // (Not on Linux, where Ctrl+Alt+←→ switch workspaces: Alt+←→ go back and forth too.)
        let back_and_forth = if cfg!(windows) {
            vec![
                KeyBinding::new("ctrl-alt-left", GoBack, workspace),
                KeyBinding::new("ctrl-alt-right", GoForward, workspace),
            ]
        } else {
            Vec::new()
        };
        back_and_forth
            .into_iter()
            .chain([
                KeyBinding::new("ctrl-w", ExpandSelection, editor),
                KeyBinding::new("ctrl-shift-w", ShrinkSelection, editor),
            ])
            .collect()
    }
}

/// Registers every shortcut for `keymap`, replacing whatever was there.
/// Counts the times the keys were bound (a keymap picked, shortcuts changed): Vim's state
/// from before is let go.
#[derive(Default)]
struct Epoch(u64);

impl gpui::Global for Epoch {}

pub fn epoch(cx: &App) -> u64 {
    cx.try_global::<Epoch>().map_or(0, |e| e.0)
}

pub fn register(keymap: Keymap, cx: &mut App) {
    let next = epoch(cx) + 1;
    cx.set_global(Epoch(next));
    cx.clear_key_bindings();
    crate::editor::bind_keys(cx);
    crate::workspace::bind_keys(cx);
    crate::file_tree::bind_keys(cx);
    crate::find_bar::bind_keys(cx);
    crate::project_search::bind_keys(cx);
    crate::terminal::bind_keys(cx);
    crate::key_prompt::bind_keys(cx);
    crate::ssh_prompt::bind_keys(cx);
    crate::trash_prompt::bind_keys(cx);
    crate::text_input::bind_keys(cx);
    crate::git_view::bind_keys(cx);
    crate::commit_page::bind_keys(cx);
    crate::github_page::bind_keys(cx);
    crate::settings_panel::bind_keys(cx);
    crate::welcome::bind_keys(cx);
    crate::editor::bind_refactor_keys(cx);
    crate::editor::bind_ai_keys(cx);
    // After the text field's keys, so ←→ can change a choice in the palette.
    crate::palette::bind_keys(cx);
    cx.bind_keys(keymap.overrides());
    crate::menus::bind_keys(cx);
    localize(cx);
    // Yours, last: they win (as you wrote or recorded them, on your keyboard).
    crate::user_keys::register(cx);
}

/// Shortcuts on digits and symbols, where the keyboard has them. They're written as on
/// an American keyboard; elsewhere a symbol can be on another key, behind Shift or AltGr,
/// or nowhere (a French AZERTY has no ` key, its digits need Shift). The Mac's own
/// shortcuts already follow the keyboard; on Windows they go to the same place as on an
/// American keyboard, as Windows' own apps have them (Ctrl+ù for the terminal on an
/// AZERTY), and on Linux the AZERTY digit row works without Shift.
fn localize(cx: &mut App) {
    if cfg!(target_os = "macos") {
        return;
    }
    let bindings: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
    let layout = cx.keyboard_layout().name().to_string();
    let mapper = cx.keyboard_mapper().clone();
    let local = if cfg!(windows) {
        bindings.iter().map(|b| in_place(b, mapper.as_ref())).filter(|b| !takes_altgr(b)).collect()
    } else {
        with_digit_row(bindings, &layout)
    };
    cx.clear_key_bindings();
    cx.bind_keys(local);
}

/// Whether `binding` is on Ctrl+Alt with a key AltGr types something with: Windows has
/// AltGr as Ctrl+Alt, so it would keep that key from typing (an AZERTY's AltGr+à is @,
/// AltGr+= is }). The action is still in the menus and the palette.
fn takes_altgr(binding: &KeyBinding) -> bool {
    binding.keystrokes().iter().any(|k| {
        let m = &k.inner().modifiers;
        m.control && m.alt && !m.platform && types_with_altgr(&k.inner().key, m.shift)
    })
}

#[cfg(windows)]
fn types_with_altgr(key: &str, shift: bool) -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn VkKeyScanW(ch: u16) -> i16;
        fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
        fn ToUnicode(key: u32, scan: u32, state: *const u8, out: *mut u16, len: i32, flags: u32) -> i32;
    }
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return false };
    let mut unit = [0u16; 2];
    let [unit] = c.encode_utf16(&mut unit) else { return false };
    // SAFETY: no pointers but ToUnicode's, to buffers of the sizes given.
    unsafe {
        let scanned = VkKeyScanW(*unit);
        if scanned == -1 {
            return false;
        }
        let key = (scanned as u16 & 0xff) as u32;
        let mut state = [0u8; 256];
        // Ctrl, Alt, left Ctrl and right Alt: what AltGr is; and Shift.
        for held in [0x11, 0x12, 0xA2, 0xA5] {
            state[held] = 0x80;
        }
        if shift {
            state[0x10] = 0x80;
            state[0xA0] = 0x80;
        }
        let mut out = [0u16; 8];
        // (Flag 4: the keyboard's state, a dead key pressed, stays as it was.)
        let typed = ToUnicode(key, MapVirtualKeyW(key, 0), state.as_ptr(), out.as_mut_ptr(), 8, 4);
        typed < 0 || (typed > 0 && char::from_u32(out[0] as u32).is_some_and(|c| !c.is_control()))
    }
}

#[cfg(not(windows))]
fn types_with_altgr(_: &str, _: bool) -> bool {
    false
}

/// `keys` as a preset's binding of them reads once localized (Windows moves symbols).
pub fn as_bound(keys: &str, cx: &App) -> String {
    let Ok(binding) = KeyBinding::load(keys, Box::new(gpui::NoAction), None, false, None, &gpui::DummyKeyboardMapper)
    else {
        return keys.to_string();
    };
    let binding = if cfg!(windows) { in_place(&binding, cx.keyboard_mapper().as_ref()) } else { binding };
    binding.keystrokes().iter().map(|k| k.unparse()).collect::<Vec<_>>().join(" ")
}

/// A key that's a digit or a symbol: where it is depends on the keyboard.
fn on_layout(key: &str) -> bool {
    let mut chars = key.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_digit() || c.is_ascii_punctuation())
}

fn source(binding: &KeyBinding) -> String {
    binding.keystrokes().iter().map(|k| k.inner().unparse()).collect::<Vec<_>>().join(" ")
}

/// `binding` on the keys at an American keyboard's places (Windows' keyboard mapper).
fn in_place(binding: &KeyBinding, mapper: &dyn gpui::PlatformKeyboardMapper) -> KeyBinding {
    if !binding.keystrokes().iter().any(|k| on_layout(&k.inner().key)) {
        return binding.clone();
    }
    let action = binding.action().boxed_clone();
    KeyBinding::load(&source(binding), action, binding.predicate(), true, binding.action_input(), mapper)
        .unwrap_or_else(|_| binding.clone())
}

/// What the digit row types without Shift on a French or Belgian AZERTY, as Linux names
/// the keys; None elsewhere (their digits need no Shift).
fn azerty_digits(layout: &str) -> Option<[&'static str; 10]> {
    let layout = layout.to_lowercase();
    let other = ["canada", "switzerland", "swiss", "bepo", "bépo", "dvorak", "qwerty", "us"];
    if other.iter().any(|o| layout.contains(o)) {
        return None;
    }
    if layout.starts_with("belgian") || layout.contains("belgium") {
        Some(["agrave", "&", "eacute", "\"", "'", "(", "section", "egrave", "!", "ccedilla"])
    } else if layout.starts_with("french") {
        Some(["agrave", "&", "eacute", "\"", "'", "(", "-", "egrave", "_", "ccedilla"])
    } else {
        None
    }
}

/// On an AZERTY, Ctrl+1 is Ctrl+Shift+&: each Ctrl+digit gets the key as typed without
/// Shift too, unless that key already does something (Ctrl+- stays smaller text).
fn with_digit_row(bindings: Vec<KeyBinding>, layout: &str) -> Vec<KeyBinding> {
    let Some(row) = azerty_digits(layout) else { return bindings };
    let taken: std::collections::HashSet<String> = bindings.iter().map(source).collect();
    let mut twins = Vec::new();
    for binding in &bindings {
        let [stroke] = binding.keystrokes() else { continue };
        let stroke = stroke.inner();
        let Some(digit) = stroke.key.chars().next().and_then(|c| c.to_digit(10)).filter(|_| stroke.key.len() == 1)
        else {
            continue;
        };
        if stroke.modifiers.shift {
            continue;
        }
        let twin = gpui::Keystroke { key: row[digit as usize].to_string(), key_char: None, ..stroke.clone() };
        let written = twin.unparse();
        if taken.contains(&written) {
            continue;
        }
        let action = binding.action().boxed_clone();
        if let Ok(twin) = KeyBinding::load(
            &written,
            action,
            binding.predicate(),
            false,
            binding.action_input(),
            &gpui::DummyKeyboardMapper,
        ) {
            twins.push(twin);
        }
    }
    bindings.into_iter().chain(twins).collect()
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

    /// Whatever the preset, typing a letter, digit or symbol types it: no key without ⌘,
    /// ⌃ or ⌥ is taken for a command where text is typed (the editor, the terminal, a field).
    #[gpui::test]
    fn typing_is_never_taken_for_a_command(cx: &mut gpui::TestAppContext) {
        let typed: Vec<String> = ('a'..='z')
            .chain('0'..='9')
            .chain("`-=[]\\;',./".chars())
            .map(|c| c.to_string())
            .chain(["space".to_string()])
            .collect();
        let places = [
            vec!["Workspace", "Editor"],
            vec!["Workspace", "Terminal"],
            vec!["Workspace", "Palette", "TextInput"],
            vec!["Workspace", "FindBar", "TextInput"],
            vec!["Workspace", "TerminalFind", "TextInput"],
        ];
        for preset in [Keymap::Null, Keymap::VsCode, Keymap::JetBrains, Keymap::Sublime, Keymap::Zed] {
            cx.update(|cx| {
                register(preset, cx);
                let keymap = cx.key_bindings();
                let keymap = keymap.borrow();
                for place in &places {
                    let context: Vec<gpui::KeyContext> =
                        place.iter().map(|c| gpui::KeyContext::parse(c).unwrap()).collect();
                    for key in &typed {
                        for stroke in [key.clone(), format!("shift-{key}")] {
                            let keystroke = gpui::Keystroke::parse(&stroke).unwrap();
                            let (bindings, _) = keymap.bindings_for_input(&[keystroke], &context);
                            let taken: Vec<&str> = bindings.iter().map(|b| b.action().name()).collect();
                            assert!(taken.is_empty(), "{preset:?}: “{stroke}” in {place:?} runs {taken:?}");
                        }
                    }
                }
            });
        }
    }

    /// The ⌃ keys of macOS text fields move the caret, except in the suggestion list,
    /// where ⌃N and ⌃P still go through the suggestions.
    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn control_keys_move_the_caret_but_not_in_suggestions(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            register(Keymap::Null, cx);
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            let run = |stroke: &str, context: &str| {
                let context: Vec<gpui::KeyContext> =
                    ["Workspace", context].iter().map(|c| gpui::KeyContext::parse(c).unwrap()).collect();
                let (bindings, _) = keymap.bindings_for_input(&[gpui::Keystroke::parse(stroke).unwrap()], &context);
                bindings.first().map(|b| b.action().name())
            };
            assert_eq!(run("ctrl-n", "Editor"), Some(crate::editor::MoveDown.name()));
            assert_eq!(run("ctrl-n", "Editor showing_completions"), Some(crate::editor::CompletionNext.name()));
            assert_eq!(run("ctrl-p", "Editor showing_completions"), Some(crate::editor::CompletionPrevious.name()));
            assert_eq!(run("ctrl-a", "TextInput"), Some(crate::text_input::Home.name()));
        });
    }

    /// ⌘E uses the selection for find, but stays the palette for JetBrains hands.
    #[cfg(target_os = "macos")]
    #[gpui::test]
    #[cfg(target_os = "macos")]
    fn command_e_finds_except_on_jetbrains(cx: &mut gpui::TestAppContext) {
        let first = |preset: Keymap, cx: &mut gpui::TestAppContext| {
            cx.update(|cx| {
                register(preset, cx);
                let keymap = cx.key_bindings();
                let keymap = keymap.borrow();
                let context: Vec<gpui::KeyContext> =
                    ["Workspace", "Editor"].iter().map(|c| gpui::KeyContext::parse(c).unwrap()).collect();
                let (bindings, _) = keymap.bindings_for_input(&[gpui::Keystroke::parse("cmd-e").unwrap()], &context);
                bindings.first().map(|b| b.action().name())
            })
        };
        assert_eq!(first(Keymap::Null, cx), Some(crate::find_bar::UseSelectionForFind.name()));
        assert_eq!(first(Keymap::JetBrains, cx), Some(TogglePalette.name()));
    }

    /// The preset's keys win over Null's own: on JetBrains, ⌘D duplicates the line.
    #[gpui::test]
    fn a_preset_overrides_the_default_keys(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            register(Keymap::JetBrains, cx);
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            let stroke = gpui::Keystroke::parse("secondary-d").unwrap();
            let context = [gpui::KeyContext::parse("Workspace").unwrap(), gpui::KeyContext::parse("Editor").unwrap()];
            let (bindings, _) = keymap.bindings_for_input(&[stroke], &context);
            assert_eq!(bindings.first().map(|b| b.action().name()), Some(DuplicateLineDown.name()));
        });
    }

    /// On a French AZERTY under Linux, Ctrl+& is Ctrl+1 (the digit needs Shift there),
    /// unless the key already does something; other keyboards keep their keys.
    #[test]
    fn an_azerty_has_its_digit_row() {
        use crate::workspace::{DecreaseFontSize, GoToTab, SplitTerminal};
        let keys = || {
            vec![
                KeyBinding::new("ctrl-1", GoToTab(1), Some("Workspace")),
                KeyBinding::new("ctrl-2", GoToTab(2), Some("Workspace")),
                KeyBinding::new("ctrl-6", GoToTab(6), Some("Workspace")),
                KeyBinding::new("ctrl--", DecreaseFontSize, Some("Workspace")),
                KeyBinding::new("ctrl-shift-5", SplitTerminal, Some("Terminal")),
            ]
        };
        let written = |bindings: Vec<KeyBinding>| bindings.iter().map(source).collect::<Vec<_>>();
        let french = written(with_digit_row(keys(), "French"));
        assert_eq!(&french[5..], ["ctrl-&", "ctrl-eacute"]);
        let belgian = written(with_digit_row(keys(), "Belgian"));
        assert!(belgian.contains(&"ctrl-section".to_string()), "{belgian:?}");
        for layout in ["English (US)", "French (Canada)", "German", "unknown"] {
            assert_eq!(with_digit_row(keys(), layout).len(), 5, "{layout}");
        }
    }

    /// Windows: a digit or symbol goes to the key in its American place on the keyboard
    /// at hand (here an AZERTY's); letters and named keys stay as they are.
    #[test]
    fn symbols_go_where_an_american_keyboard_has_them() {
        struct Azerty;
        impl gpui::PlatformKeyboardMapper for Azerty {
            fn map_key_equivalent(
                &self,
                mut keystroke: gpui::Keystroke,
                use_key_equivalents: bool,
            ) -> gpui::KeybindingKeystroke {
                if use_key_equivalents {
                    let place = match keystroke.key.as_str() {
                        "1" => "&",
                        "`" => "ù",
                        "/" => ":",
                        other => other,
                    };
                    keystroke.key = place.to_string();
                }
                gpui::KeybindingKeystroke::from_keystroke(keystroke)
            }
            fn get_key_equivalents(&self) -> Option<&gpui_collections::HashMap<char, char>> {
                None
            }
        }
        let placed = |keys: &str| {
            let binding = KeyBinding::new(keys, ToggleTerminal, Some("Workspace"));
            source(&in_place(&binding, &Azerty))
        };
        assert_eq!(placed("ctrl-`"), "ctrl-ù");
        assert_eq!(placed("ctrl-1"), "ctrl-&");
        assert_eq!(placed("ctrl-k ctrl-/"), "ctrl-k ctrl-:");
        assert_eq!(placed("ctrl-j"), "ctrl-j");
        assert_eq!(placed("ctrl-shift-pageup"), "ctrl-shift-pageup");
        assert!(on_layout("1") && on_layout("`") && !on_layout("a") && !on_layout("f1"));
    }
}
