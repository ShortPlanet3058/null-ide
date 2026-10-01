mod assets;
mod buffer;
mod editor;
mod element;
mod file_tree;
mod fuzzy;
mod highlight;
mod menus;
mod palette;
mod text_input;
mod theme;
mod workspace;

use gpui::{
    App, Application, Bounds, Focusable, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, point, prelude::*,
    px, size,
};
use menus::Quit;
use std::path::PathBuf;
use workspace::Workspace;

/// `null` opens the current folder, `null <folder>` opens that folder, and
/// `null <file>` opens the file inside the current folder (or its own folder
/// when it lives elsewhere).
fn resolve_args() -> (PathBuf, Option<PathBuf>) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let Some(arg) = std::env::args().nth(1) else { return (cwd, None) };
    let path = cwd.join(arg);
    if path.is_dir() {
        return (path, None);
    }
    let root = if path.starts_with(&cwd) { cwd } else { path.parent().map(PathBuf::from).unwrap_or(cwd) };
    (root, Some(path))
}

fn main() {
    let (root, file) = resolve_args();

    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        theme::init(cx);
        editor::bind_keys(cx);
        workspace::bind_keys(cx);
        palette::bind_keys(cx);
        text_input::bind_keys(cx);
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
            cx.new(|cx| {
                let mut workspace = Workspace::new(root, window, cx);
                match file {
                    Some(file) => workspace.open_file(file, window, cx),
                    None => window.focus(&workspace.focus_handle(cx)),
                }
                workspace
            })
        })
        .expect("failed to open the main window");
        cx.activate(true);
    });
}
