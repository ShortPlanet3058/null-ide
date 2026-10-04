//! Shared sizes and small pieces, so every panel lines up: one set of corner radii, text
//! sizes and row heights, and one key cap, switch and section heading for the whole app.
//!
//! Colour rule: `faint` is for decoration only; any text meant to be read uses `muted`.

use crate::theme::{Syntax, Theme, ThemeName};
use gpui::{AnyElement, AppContext as _, Div, FontWeight, IntoElement, ParentElement, SharedString, Styled, div, px};

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

/// A key or shortcut, like ⌘K: small, on the surface colour, with a border that reads.
pub fn key_cap(keys: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(R_KEY))
        .border_1()
        .border_color(theme.line_strong)
        .bg(theme.surface)
        .text_size(px(T_XS))
        .text_color(theme.muted)
        .child(keys.into())
        .into_any_element()
}

/// An on/off switch, the same everywhere (Settings, ⌘K).
pub fn switch(on: bool, theme: &Theme) -> Div {
    div()
        .flex_none()
        .w(px(32.))
        .h(px(18.))
        .p(px(2.))
        .rounded_full()
        .flex()
        .when_on(on, theme)
        .child(div().size(px(14.)).rounded_full().bg(if on { theme.on_accent } else { theme.muted }))
}

trait SwitchTrack {
    fn when_on(self, on: bool, theme: &Theme) -> Self;
}

impl SwitchTrack for Div {
    fn when_on(self, on: bool, theme: &Theme) -> Self {
        if on { self.justify_end().bg(theme.caret) } else { self.bg(theme.line_strong) }
    }
}

/// A heading over a group of rows, in small capitals.
pub fn section_heading(text: &str, theme: &Theme) -> Div {
    div().text_size(px(T_XS)).font_weight(FontWeight::SEMIBOLD).text_color(theme.faint).child(text.to_uppercase())
}

/// The track of a segmented choice (theme, sidebar view…); pieces go inside it.
pub fn segmented(theme: &Theme) -> Div {
    div().flex().items_center().gap(px(2.)).p(px(2.)).rounded(px(R_CONTROL)).bg(theme.sunken)
}

/// One piece of a segmented choice.
pub fn segment(active: bool, theme: &Theme) -> Div {
    let piece = div().px(px(10.)).py(px(3.)).rounded(px(R_CONTROL - 2.)).text_size(px(T_SM));
    if active { piece.bg(theme.hairline).text_color(theme.foreground) } else { piece.text_color(theme.muted) }
}

/// A theme as a small card: its background with a few lines of coloured "code", and its
/// name under it. The same on the welcome screen and in Settings.
pub fn theme_preview(name: ThemeName, active: bool, theme: &Theme) -> Div {
    theme_preview_scaled(name, active, theme, 1.)
}

/// The theme card at `scale` times its usual size (bigger on the welcome screen).
pub fn theme_preview_scaled(name: ThemeName, active: bool, theme: &Theme, scale: f32) -> Div {
    let preview = Theme::named(name);
    let k = |v: f32| px(v * scale);
    let bar = |w: f32, color: gpui::Hsla| div().h(k(4.)).w(k(w)).rounded(k(2.)).bg(color);
    let line = |indent: f32| div().flex().gap(k(5.)).pl(k(indent));
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .w(k(120.))
                .h(k(70.))
                .p(k(11.))
                .flex()
                .flex_col()
                .gap(k(7.))
                .rounded(px(R_POPOVER * scale.sqrt()))
                .bg(preview.background)
                .border_2()
                .border_color(if active { theme.caret } else { theme.line_strong })
                .child(
                    line(0.)
                        .child(bar(20., preview.syntax(Syntax::Keyword)))
                        .child(bar(38., preview.syntax(Syntax::Function))),
                )
                .child(line(10.).child(bar(28., preview.foreground)).child(bar(32., preview.syntax(Syntax::String))))
                .child(line(10.).child(bar(16., preview.syntax(Syntax::Comment))).child(bar(2., preview.caret))),
        )
        .child(
            div()
                .text_size(px(T_MD))
                .text_color(if active { theme.foreground } else { theme.muted })
                .child(name.label()),
        )
}

/// A tooltip: what a control does, and its shortcut when it has one. Small and quiet,
/// for controls that show only an icon or a symbol.
pub struct Tip {
    label: SharedString,
    keys: Option<String>,
}

impl gpui::Render for Tip {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        let theme = cx.global::<Theme>();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(R_ROW))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.line_strong)
            .shadow_md()
            .font_family(cx.global::<crate::fonts::Fonts>().ui.clone())
            .text_size(px(T_SM))
            .text_color(theme.foreground)
            .child(self.label.clone())
            .children(self.keys.clone().map(|k| div().text_color(theme.muted).child(k)))
    }
}

/// Builds a tooltip for `.tooltip(...)`: `label`, and the keys bound to `action` (read when
/// it shows, so another keymap shows its own).
pub fn tip(
    label: &'static str,
    action: Option<Box<dyn gpui::Action>>,
) -> impl Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView + 'static {
    move |_, cx| {
        let keys = action.as_ref().and_then(|a| crate::palette::shortcut(a.as_ref(), cx));
        cx.new(|_| Tip { label: label.into(), keys }).into()
    }
}
