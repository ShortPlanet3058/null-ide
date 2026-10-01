use crate::editor::{Copy, Cut, Paste, Redo, Save, SelectAll, Undo};
use crate::find_bar::{DeployFind, DeployReplace, FindNext, FindPrevious};
use crate::workspace::{
    CloseTab, DecreaseFontSize, IncreaseFontSize, NextTab, Open, OpenSettings, PreviousTab, ResetFontSize,
    TogglePalette, ToggleSidebar, UseGraphiteTheme, UseOledTheme, UsePaperTheme,
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
                MenuItem::action("Open…", Open),
                MenuItem::separator(),
                MenuItem::action("Save", Save),
                MenuItem::action("Close Tab", CloseTab),
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
                MenuItem::action("Find…", DeployFind),
                MenuItem::action("Find Next", FindNext),
                MenuItem::action("Find Previous", FindPrevious),
                MenuItem::action("Replace…", DeployReplace),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Command Palette…", TogglePalette),
                MenuItem::separator(),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
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
