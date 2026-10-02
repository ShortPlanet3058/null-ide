use crate::editor::{
    Copy, Cut, DeleteLine, DuplicateLineDown, GoToDefinition, Indent, MoveLineDown, MoveLineUp, Outdent, Paste, Redo,
    Save, SelectAll, SelectLine, ShowInfo, ToggleComment, Undo,
};
use crate::find_bar::{DeployFind, DeployReplace, FindNext, FindPrevious};
use crate::workspace::{
    CloseAllTabs, CloseTab, DecreaseFontSize, GoToLine, IncreaseFontSize, NewUntitled, NextTab, Open, OpenSettings, PreviousTab,
    ReopenClosedTab, ResetFontSize, SaveAll, SaveAs, SearchProject, TogglePalette, ToggleSidebar, ToggleTerminal,
    UseGraphiteTheme, UseOledTheme, UsePaperTheme,
};
use gpui::{App, Menu, MenuItem, SystemMenuType, actions};

actions!(null, [Quit, ToggleFadeWhileTyping]);

/// Installs the menu bar. Menu items can't show a checkmark yet, so the fade
/// toggle's label says what choosing it will do.
pub fn set(cx: &mut App, fade_while_typing: bool) {
    let fade_label = if fade_while_typing { "Stop Fading Bars While Typing" } else { "Fade Bars While Typing" };
    cx.set_menus(vec![
        Menu {
            name: "Null".into(),
            items: vec![
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit Null", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New File", NewUntitled),
                MenuItem::action("Open…", Open),
                MenuItem::action("Reopen Closed Tab", ReopenClosedTab),
                MenuItem::separator(),
                MenuItem::action("Save", Save),
                MenuItem::action("Save As…", SaveAs),
                MenuItem::action("Save All", SaveAll),
                MenuItem::action("Close Tab", CloseTab),
                MenuItem::action("Close All Tabs", CloseAllTabs),
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
                        MenuItem::action("Go to Line…", GoToLine),
                    ],
                }),
                MenuItem::separator(),
                MenuItem::action("Find…", DeployFind),
                MenuItem::action("Find Next", FindNext),
                MenuItem::action("Find Previous", FindPrevious),
                MenuItem::action("Replace…", DeployReplace),
                MenuItem::action("Search in Project…", SearchProject),
                MenuItem::separator(),
                MenuItem::action("Go to Definition", GoToDefinition),
                MenuItem::action("Show Info at Cursor", ShowInfo),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Command Palette…", TogglePalette),
                MenuItem::separator(),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Toggle Terminal", ToggleTerminal),
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PreviousTab),
                MenuItem::separator(),
                MenuItem::action("Bigger Text", IncreaseFontSize),
                MenuItem::action("Smaller Text", DecreaseFontSize),
                MenuItem::action("Actual Size", ResetFontSize),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: "Theme".into(),
                    items: vec![
                        MenuItem::action("OLED", UseOledTheme),
                        MenuItem::action("Graphite", UseGraphiteTheme),
                        MenuItem::action("Paper", UsePaperTheme),
                    ],
                }),
                MenuItem::action(fade_label, ToggleFadeWhileTyping),
            ],
        },
    ]);
}
