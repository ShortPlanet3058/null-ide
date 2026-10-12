//! The window's own frame where the system leaves it to Null. The Mac draws its buttons in
//! Null's title bar; Windows (its title bar hidden, the same look) and Linux with
//! client-side decorations (Wayland, mostly) get minimise, maximise and close at the bar's
//! right end, from here, and on Linux edges to resize the window by.

use crate::theme::Theme;
use gpui::{
    AnyElement, App, ClickEvent, CursorStyle, Decorations, MouseButton, ResizeEdge, Window, WindowControlArea, div,
    prelude::*, px,
};

/// Whether Null draws the window's buttons itself.
pub fn draws_controls(window: &Window) -> bool {
    if cfg!(target_os = "macos") {
        false
    } else if cfg!(windows) {
        true
    } else {
        matches!(window.window_decorations(), Decorations::Client { .. })
    }
}

/// Each button's width (Windows' own are about this).
const BUTTON: f32 = 44.;

/// Minimise, maximise (or restore) and close, for the title bar's right end, its height.
/// `close` runs Null's own closing (asking about unsaved work first).
pub fn controls(window: &Window, theme: &Theme, close: impl Fn(&mut Window, &mut App) + 'static) -> AnyElement {
    let maximized = window.is_maximized();
    let button = |id: &'static str, icon: &'static str, area: WindowControlArea, danger: bool| {
        let (hover, icon_hover) =
            if danger { (theme.error, theme.on_accent) } else { (crate::ui::hover(theme), theme.foreground) };
        div()
            .id(id)
            .group(id)
            .w(px(BUTTON))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .hover(move |d| d.bg(hover))
            // Windows works these out itself (as its own buttons: snap layouts and all).
            .window_control_area(area)
            .child(
                gpui::svg()
                    .path(icon)
                    .size(px(12.))
                    .text_color(theme.muted)
                    .group_hover(id, move |s| s.text_color(icon_hover)),
            )
    };
    let maximize = if maximized { "icons/window-restore.svg" } else { "icons/window-maximize.svg" };
    let row = div()
        .flex_none()
        .h_full()
        .flex()
        .child(
            button("window-minimize", "icons/window-minimize.svg", WindowControlArea::Min, false)
                .when(!cfg!(windows), |b| b.on_click(|_: &ClickEvent, window, _| window.minimize_window())),
        )
        .child(
            button("window-maximize", maximize, WindowControlArea::Max, false)
                .when(!cfg!(windows), |b| b.on_click(|_: &ClickEvent, window, _| window.zoom_window())),
        )
        .child(
            button("window-close", "icons/x.svg", WindowControlArea::Close, true)
                .when(!cfg!(windows), |b| b.on_click(move |_: &ClickEvent, window, cx| close(window, cx))),
        );
    row.into_any_element()
}

/// On Linux with client-side decorations, not maximised or tiled: thin edges and corners
/// to resize the window by, laid over its sides (the root positioned `relative`).
pub fn resize_edges(window: &Window) -> Vec<AnyElement> {
    const EDGE: f32 = 5.;
    const CORNER: f32 = 12.;
    let Decorations::Client { tiling } = window.window_decorations() else { return Vec::new() };
    if cfg!(target_os = "macos") || cfg!(windows) || window.is_maximized() || tiling.top && tiling.bottom {
        return Vec::new();
    }
    let edge = |id: &'static str, edge: ResizeEdge, cursor: CursorStyle| {
        div().id(id).absolute().cursor(cursor).on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            window.start_window_resize(edge);
        })
    };
    let ns = CursorStyle::ResizeUpDown;
    let ew = CursorStyle::ResizeLeftRight;
    let nwse = CursorStyle::ResizeUpLeftDownRight;
    let nesw = CursorStyle::ResizeUpRightDownLeft;
    vec![
        edge("edge-top", ResizeEdge::Top, ns).top_0().left(px(CORNER)).right(px(CORNER)).h(px(EDGE)),
        edge("edge-bottom", ResizeEdge::Bottom, ns).bottom_0().left(px(CORNER)).right(px(CORNER)).h(px(EDGE)),
        edge("edge-left", ResizeEdge::Left, ew).left_0().top(px(CORNER)).bottom(px(CORNER)).w(px(EDGE)),
        edge("edge-right", ResizeEdge::Right, ew).right_0().top(px(CORNER)).bottom(px(CORNER)).w(px(EDGE)),
        edge("corner-top-left", ResizeEdge::TopLeft, nwse).top_0().left_0().size(px(CORNER)),
        edge("corner-bottom-right", ResizeEdge::BottomRight, nwse).bottom_0().right_0().size(px(CORNER)),
        edge("corner-top-right", ResizeEdge::TopRight, nesw).top_0().right_0().size(px(CORNER)),
        edge("corner-bottom-left", ResizeEdge::BottomLeft, nesw).bottom_0().left_0().size(px(CORNER)),
    ]
    .into_iter()
    .map(IntoElement::into_any_element)
    .collect()
}
