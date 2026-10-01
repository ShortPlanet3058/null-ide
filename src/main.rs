mod buffer;
mod editor;
mod element;
mod highlight;
mod menus;
mod theme;
mod workspace;

use editor::Editor;
use gpui::{
    App, Application, Bounds, Focusable, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, point, prelude::*,
    px, size,
};
use menus::Quit;
use std::path::PathBuf;
use workspace::Workspace;

fn main() {
    let path = std::env::args().nth(1).map(PathBuf::from);

    Application::new().run(move |cx: &mut App| {
        theme::init(cx);
        editor::bind_keys(cx);
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        menus::set(cx, false);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Null".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.), px(13.))),
            }),
            window_min_size: Some(size(px(480.), px(320.))),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            let editor = cx.new(|cx| match path {
                Some(path) => Editor::open(path, cx),
                None => Editor::new(Default::default(), None, cx),
            });
            window.focus(&editor.focus_handle(cx));
            cx.new(|cx| Workspace::new(editor, cx))
        })
        .expect("failed to open the main window");
        cx.activate(true);
    });
}
