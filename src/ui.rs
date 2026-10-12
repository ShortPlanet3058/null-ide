//! Shared sizes and small pieces, so every panel lines up: one set of corner radii, text
//! sizes and row heights, and one key cap, switch and section heading for the whole app.
//!
//! Colour rule: `faint` is for decoration only; any text meant to be read uses `muted`.

use crate::theme::{Syntax, Theme, ThemeName};
use gpui::{AnyElement, AppContext as _, Div, FontWeight, IntoElement, ParentElement, SharedString, Styled, div};

/// The size of the interface's lengths (a rem) at 100%: View > Zoom changes the window's.
pub const BASE_REM: f32 = 16.;

/// How big the interface is now (1: 100%).
pub fn zoom(cx: &gpui::App) -> f32 {
    use crate::settings::{MAX_ZOOM, MIN_ZOOM, Settings};
    cx.try_global::<Settings>().map_or(1., |s| s.ui_zoom).clamp(MIN_ZOOM, MAX_ZOOM)
}

/// `points` at 100%, in real pixels at the interface's zoom: for sizes worked out beside the
/// window's own (a panel's width taken from the window's).
pub fn zoomed(points: f32, cx: &gpui::App) -> f32 {
    points * zoom(cx)
}

/// Sizes the window's interface lengths (`px` below) for the zoom: each window, as it draws.
pub fn apply_zoom(window: &mut gpui::Window, cx: &gpui::App) {
    let rem = gpui::px(BASE_REM * zoom(cx));
    if window.rem_size() != rem {
        window.set_rem_size(rem);
    }
}

/// A length of the interface, as it is at 100%: it grows and shrinks with the interface's
/// zoom (the window's rem size). The text being written (editor, terminal) has its own
/// size, in real pixels (`gpui::px`).
pub fn px(points: f32) -> gpui::Rems {
    gpui::rems(points / BASE_REM)
}

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

/// The keys that run `action` now (this keymap's, or the person's own), else `fallback`:
/// what a label or a notice names, so it's right on every platform and preset.
pub fn keys(action: &dyn gpui::Action, fallback: &str, cx: &gpui::App) -> String {
    crate::palette::shortcut(action, cx).unwrap_or_else(|| fallback.to_string())
}

/// "⌘. resolves each", with the keys that do.
pub fn resolve_each(cx: &gpui::App) -> String {
    format!("{} resolves each", keys(&crate::editor::QuickFix, "⌘.", cx))
}

/// The colour under a row (or a quiet control) the mouse is over.
pub fn hover(theme: &Theme) -> gpui::Hsla {
    theme.hairline.opacity(0.6)
}

/// Where content starts in the sidebar, from its edge.
pub const INSET: f32 = 16.;

/// A quiet button: a bordered label, the same everywhere (Settings, git, pages).
pub fn button(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, theme: &Theme) -> gpui::Stateful<Div> {
    use gpui::{InteractiveElement as _, StatefulInteractiveElement as _};
    div()
        .id(id)
        .flex_none()
        .px(px(10.))
        .h(px(CONTROL))
        .flex()
        .items_center()
        .rounded(px(R_CONTROL))
        .border_1()
        .border_color(theme.line_strong)
        .text_size(px(T_SM))
        .text_color(theme.foreground)
        .cursor_pointer()
        .hover(|d| d.bg(hover(theme)))
        .active(|d| d.opacity(0.7))
        .child(label.into())
}

/// The one main action of a place, in the accent; dimmed and inert when it can't be done.
pub fn primary_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    use gpui::{InteractiveElement as _, StatefulInteractiveElement as _};
    let button = div()
        .id(id)
        .flex_none()
        .px(px(12.))
        .h(px(CONTROL))
        .flex()
        .items_center()
        .rounded(px(R_CONTROL))
        .text_size(px(T_SM))
        .child(label.into());
    if enabled {
        button
            .bg(theme.caret)
            .text_color(theme.on_accent)
            .cursor_pointer()
            .hover(|d| d.opacity(0.9))
            .active(|d| d.opacity(0.7))
    } else {
        button.bg(theme.hairline).text_color(theme.faint)
    }
}

/// A word that does something, without a frame (a list's own small actions).
pub fn link(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, theme: &Theme) -> gpui::Stateful<Div> {
    use gpui::{InteractiveElement as _, StatefulInteractiveElement as _};
    div()
        .id(id)
        .flex_none()
        .text_color(theme.muted)
        .cursor_pointer()
        .hover(|d| d.text_color(theme.foreground))
        .active(|d| d.opacity(0.7))
        .child(label.into())
}

/// A small square button holding an icon (close, add…), with what it does on hover.
pub fn icon_button(
    id: impl Into<gpui::ElementId>,
    icon: &'static str,
    tip_label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    use gpui::{InteractiveElement as _, StatefulInteractiveElement as _};
    div()
        .id(id)
        .flex_none()
        .size(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_KEY))
        .cursor_pointer()
        .hover(|d| d.bg(hover(theme)))
        .active(|d| d.opacity(0.7))
        .tooltip(tip(tip_label, None))
        .child(gpui::svg().path(icon).size(px(12.)).text_color(theme.muted))
}

/// A text field's box (its input goes inside): one look for every field.
pub fn field(theme: &Theme) -> Div {
    div()
        .h(px(FIELD))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded(px(R_CONTROL))
        .border_1()
        .border_color(theme.hairline)
        .bg(theme.background)
        .text_size(px(T_MD))
}

/// A heading over a group of rows in the sidebar, at its content's edge.
pub fn sidebar_heading(text: &str, theme: &Theme) -> Div {
    section_heading(text, theme).px(px(INSET)).pt(px(14.)).pb(px(4.))
}

/// What a list says when it has nothing to show.
pub fn empty_state(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div().px(px(INSET)).pt(px(12.)).text_size(px(T_SM)).text_color(theme.muted).child(text.into())
}

/// A raised card (a branch, a pull request) on the sidebar or a page.
pub fn card(theme: &Theme) -> Div {
    div().rounded(px(R_ROW_LG)).bg(theme.raised)
}

/// A box ticked, half-ticked (`partly`: some of it), or empty.
pub fn checkbox(on: bool, partly: bool, theme: &Theme) -> Div {
    let box_ = div().flex_none().size(px(14.)).rounded(px(R_KEY)).flex().items_center().justify_center();
    if on {
        box_.bg(theme.caret).text_color(theme.on_accent).text_size(px(10.)).child(if partly { "–" } else { "✓" })
    } else {
        box_.border_1().border_color(theme.line_strong)
    }
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
    label: impl Into<SharedString>,
    action: Option<Box<dyn gpui::Action>>,
) -> impl Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView + 'static {
    let label = label.into();
    move |_, cx| {
        let keys = action.as_ref().and_then(|a| crate::palette::shortcut(a.as_ref(), cx));
        cx.new(|_| Tip { label: label.clone(), keys }).into()
    }
}

/// The character boundary nearest `x` on a shaped line. GPUI's own puts anything past the
/// start of the last character at the line's end, so a click on the left half of a line's
/// last letter landed after it; here it lands before, as anywhere else on the line.
pub fn index_at_x(line: &gpui::LineLayout, x: gpui::Pixels) -> usize {
    let index = line.closest_index_for_x(x);
    if index != line.len {
        return index;
    }
    let last = line.runs.iter().flat_map(|r| r.glyphs.iter()).next_back();
    match last {
        Some(glyph) if x < (glyph.position.x + line.width) / 2. => glyph.index,
        _ => index,
    }
}
