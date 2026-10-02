//! Shared sizes, so every panel lines up: one set of corner radii, text sizes and row
//! heights for the whole app.

// Corner radii: a row inside a panel is rounded by the panel's radius minus its padding.
/// Key caps, tiny buttons.
pub const R_KEY: f32 = 4.;
/// Rows in popovers (menus, the suggestion list).
pub const R_ROW: f32 = 6.;
/// Buttons, fields, tabs, toggles, choices.
pub const R_CONTROL: f32 = 7.;
/// Rows in the palette and Settings.
pub const R_ROW_LG: f32 = 8.;
/// Popovers: the suggestion list, hover cards, menus, the find bar, notices.
pub const R_POPOVER: f32 = 10.;
/// The palette, Settings, the welcome, the key prompt.
pub const R_MODAL: f32 = 14.;

// Text sizes.
/// Section headings (capitals), key caps.
pub const T_XS: f32 = 11.;
/// Details, hints, paths, the status bar.
pub const T_SM: f32 = 12.;
/// Lists, menus, tabs, buttons, fields.
pub const T_MD: f32 = 13.;
/// Palette rows, Settings titles.
pub const T_LG: f32 = 14.;
/// The palette's field, panel titles.
pub const T_XL: f32 = 17.;

// Heights.
/// The suggestion list, search matches.
pub const ROW_SM: f32 = 24.;
/// The file tree, menus.
pub const ROW: f32 = 26.;
/// The palette.
pub const ROW_LG: f32 = 34.;
/// Buttons and toggles; fields are a little taller.
pub const CONTROL: f32 = 26.;
pub const FIELD: f32 = 28.;
