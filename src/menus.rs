use crate::editor::{
    AddCursorAbove, AddCursorBelow, AddNextOccurrence, Copy, Cut, DeleteLine, DuplicateLineDown, GoToDefinition,
    Indent, MoveLineDown, MoveLineUp, Outdent, Paste, Redo, Save, SelectAll, SelectAllOccurrences, SelectLine,
    ShowInfo, ToggleComment, Undo,
};
use crate::find_bar::{DeployFind, DeployReplace, FindNext, FindPrevious};
use crate::workspace::{
    CloseAllTabs, CloseTab, DecreaseFontSize, GoToLine, IncreaseFontSize, NewUntitled, NextTab, Open, OpenSettings,
    PreviousTab, ReopenClosedTab, ResetFontSize, SaveAll, SaveAs, SearchProject, ShowCommands, TogglePalette,
    ToggleSidebar, ToggleTerminal,
};
use gpui::{App, KeyBinding, Menu, MenuItem, SystemMenuType, actions};

actions!(null, [Quit, ToggleFadeWhileTyping, ToggleWordWrap, Hide, HideOthers, ShowAll, Minimize, Zoom]);

/// The menu items every Mac app has: hiding the app, and the Window menu's two.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            window.update(cx, |_, window, _| window.minimize_window()).ok();
        }
    });
    cx.on_action(|_: &Zoom, cx| {
        if let Some(window) = cx.active_window() {
            window.update(cx, |_, window, _| window.zoom_window()).ok();
        }
    });
}

/// The app's own keys, the same whatever the keymap preset.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-m", Minimize, None),
        ]);
    }
}

/// Installs the menu bar. Menu items can't show a checkmark yet, so toggles
/// are labelled with what choosing them will do.
pub fn set(cx: &mut App) {
    let settings = cx.global::<crate::settings::Settings>();
    let fade_label =
        if settings.fade_bars_while_typing { "Stop Fading Bars While Typing" } else { "Fade Bars While Typing" };
    let wrap_label = if settings.word_wrap { "Stop Wrapping Lines" } else { "Wrap Lines" };
    cx.set_menus(vec![
        Menu {
            name: "Null".into(),
            items: vec![
                MenuItem::action("Welcome to Null…", crate::workspace::ShowWelcome),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::action("Install Shell Command", crate::workspace::InstallShellCommand),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Null", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Null", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New File", NewUntitled),
                MenuItem::action("Open File or Folder…", Open),
                MenuItem::action("Open Recent…", crate::workspace::OpenRecent),
                MenuItem::action("New Window…", crate::workspace::NewWindow),
                MenuItem::action("Reopen Closed Tab", ReopenClosedTab),
                MenuItem::separator(),
                MenuItem::action("Save", Save),
                MenuItem::action("Save As…", SaveAs),
                MenuItem::action("Save All", SaveAll),
                MenuItem::action("Close Tab", CloseTab),
                MenuItem::action("Close All Tabs", CloseAllTabs),
                MenuItem::separator(),
                MenuItem::action("Review Changes…", crate::workspace::ReviewChanges),
                MenuItem::action("Commit…", crate::workspace::CommitAll),
                MenuItem::action("Push", crate::workspace::PushBranch),
                MenuItem::action("Switch Branch…", crate::workspace::SwitchBranch),
                MenuItem::action("Revert All Changes…", crate::workspace::RevertAllChanges),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo", Undo),
                MenuItem::action("Redo", Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", Cut),
                MenuItem::action("Copy", Copy),
                MenuItem::action("Paste", Paste),
                MenuItem::action("Select All", SelectAll),
                MenuItem::action("Expand Selection", crate::editor::ExpandSelection),
                MenuItem::action("Shrink Selection", crate::editor::ShrinkSelection),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: "Lines".into(),
                    items: vec![
                        // Tab / Shift+Tab work on any selection; the menu shows the other shortcut.
                        MenuItem::action("Indent (Tab on a selection)", Indent),
                        MenuItem::action("Outdent (Shift+Tab)", Outdent),
                        MenuItem::action("Toggle Comment", ToggleComment),
                        MenuItem::separator(),
                        MenuItem::action("Move Line Up", MoveLineUp),
                        MenuItem::action("Move Line Down", MoveLineDown),
                        MenuItem::action("Duplicate Line", DuplicateLineDown),
                        MenuItem::action("Delete Line", DeleteLine),
                        MenuItem::action("Select Line", SelectLine),
                        MenuItem::separator(),
                        MenuItem::action("Insert Line Below", crate::editor::NewlineBelow),
                        MenuItem::action("Insert Line Above", crate::editor::NewlineAbove),
                        MenuItem::action("Join Lines", crate::editor::JoinLines),
                        MenuItem::action("Sort Lines", crate::editor::SortLines),
                        MenuItem::separator(),
                        MenuItem::action("Upper Case", crate::editor::UpperCase),
                        MenuItem::action("Lower Case", crate::editor::LowerCase),
                    ],
                }),
                MenuItem::submenu(Menu {
                    name: "Folding".into(),
                    items: vec![
                        MenuItem::action("Fold", crate::editor::Fold),
                        MenuItem::action("Unfold", crate::editor::Unfold),
                        MenuItem::separator(),
                        MenuItem::action("Fold All", crate::editor::FoldAll),
                        MenuItem::action("Unfold All", crate::editor::UnfoldAll),
                    ],
                }),
                MenuItem::submenu(Menu {
                    name: "Cursors".into(),
                    items: vec![
                        // Cmd+Shift+click in the text adds (or removes) a cursor too.
                        MenuItem::action("Add Next Occurrence", AddNextOccurrence),
                        MenuItem::action("Select All Occurrences", SelectAllOccurrences),
                        MenuItem::action("Add Cursor Above", AddCursorAbove),
                        MenuItem::action("Add Cursor Below", AddCursorBelow),
                        MenuItem::action("Add Cursors to Line Ends", crate::editor::AddCursorsToLineEnds),
                        MenuItem::action("Undo Last Cursor", crate::editor::UndoCursor),
                    ],
                }),
                MenuItem::separator(),
                MenuItem::action("Find…", DeployFind),
                MenuItem::action("Find Next", FindNext),
                MenuItem::action("Find Previous", FindPrevious),
                MenuItem::action("Replace…", DeployReplace),
                MenuItem::action("Search in Project…", SearchProject),
                MenuItem::action("Replace in Project…", crate::workspace::ReplaceInProject),
                MenuItem::separator(),
                MenuItem::action("Back", crate::workspace::GoBack),
                MenuItem::action("Forward", crate::workspace::GoForward),
                MenuItem::action("Go to Matching Bracket", crate::editor::GoToMatchingBracket),
                MenuItem::separator(),
                MenuItem::action("Go to Definition", GoToDefinition),
                MenuItem::action("Go to Type Definition", crate::editor::GoToTypeDefinition),
                MenuItem::action("Go to Implementation", crate::editor::GoToImplementation),
                MenuItem::action("Find References", crate::editor::FindReferences),
                MenuItem::action("Rename Symbol", crate::editor::RenameSymbol),
                MenuItem::action("Quick Fix…", crate::editor::QuickFix),
                MenuItem::action("Format Document", crate::editor::FormatDocument),
                MenuItem::action("Format Selection", crate::editor::FormatSelection),
                MenuItem::action("Show Problems", crate::workspace::ShowProblems),
                MenuItem::action("Next Problem", crate::workspace::NextProblem),
                MenuItem::action("Previous Problem", crate::workspace::PreviousProblem),
                MenuItem::action("Next Change", crate::editor::NextChange),
                MenuItem::action("Previous Change", crate::editor::PreviousChange),
                MenuItem::action("Next Conflict", crate::editor::NextConflict),
                MenuItem::action("Previous Conflict", crate::editor::PreviousConflict),
                MenuItem::action("Show Info at Cursor", ShowInfo),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Quick Settings and Commands…", ShowCommands),
                MenuItem::action("Go to File…", TogglePalette),
                MenuItem::action("Go to Symbol…", crate::workspace::GoToSymbol),
                MenuItem::action("Go to Symbol in Project…", crate::workspace::GoToSymbolInProject),
                MenuItem::action("Go to Line…", GoToLine),
                MenuItem::separator(),
                MenuItem::action("Markdown Preview", crate::editor::ToggleMarkdownPreview),
                MenuItem::action("Markdown Preview to the Side", crate::workspace::OpenPreviewToTheSide),
                MenuItem::separator(),
                MenuItem::action("Focus Mode", crate::workspace::ToggleFocusMode),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Toggle Terminal", ToggleTerminal),
                MenuItem::action("Run Task…", crate::workspace::RunTask),
                MenuItem::action("Run Test at Cursor", crate::workspace::RunTestAtCursor),
                MenuItem::action("Run Tests in File", crate::workspace::RunTestsInFile),
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Move Tab to the Right Side", crate::workspace::MoveTabRight),
                MenuItem::action("Move Tab to the Left Side", crate::workspace::MoveTabLeft),
                MenuItem::action("Open on the Other Side Too", crate::workspace::OpenOnOtherSide),
                MenuItem::action("Previous Tab", PreviousTab),
                MenuItem::separator(),
                MenuItem::action("Bigger Text", IncreaseFontSize),
                MenuItem::action("Smaller Text", DecreaseFontSize),
                MenuItem::action("Actual Size", ResetFontSize),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: "Theme".into(),
                    items: vec![
                        MenuItem::action("Null", crate::workspace::UseNullTheme),
                        MenuItem::action("Ash", crate::workspace::UseAshTheme),
                        MenuItem::action("Midnight", crate::workspace::UseMidnightTheme),
                        MenuItem::action("Moss", crate::workspace::UseMossTheme),
                        MenuItem::separator(),
                        MenuItem::action("Paper", crate::workspace::UsePaperTheme),
                        MenuItem::action("Dune", crate::workspace::UseDuneTheme),
                    ],
                }),
                MenuItem::action(wrap_label, ToggleWordWrap),
                MenuItem::action(fade_label, ToggleFadeWhileTyping),
            ],
        },
        Menu {
            name: "Debug".into(),
            items: vec![
                MenuItem::action("Start or Continue", crate::workspace::StartDebugging),
                MenuItem::action("Pause", crate::workspace::PauseDebugging),
                MenuItem::action("Stop", crate::workspace::StopDebugging),
                MenuItem::separator(),
                MenuItem::action("Step Over", crate::workspace::StepOver),
                MenuItem::action("Step Into", crate::workspace::StepInto),
                MenuItem::action("Step Out", crate::workspace::StepOut),
                MenuItem::separator(),
                MenuItem::action("Toggle Breakpoint", crate::editor::ToggleBreakpoint),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![MenuItem::action("Minimize", Minimize), MenuItem::action("Zoom", Zoom)],
        },
    ]);
}
