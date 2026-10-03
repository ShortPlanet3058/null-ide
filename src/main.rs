mod ai;
mod assets;
mod buffer;
mod editor;
mod element;
mod file_tree;
mod find_bar;
mod fonts;
mod fs_ops;
mod fuzzy;
mod git;
mod highlight;
mod key_prompt;
mod keymap;
mod languages;
mod lsp;
mod lsp_store;
mod markdown;
mod menus;
mod palette;
mod project_index;
mod project_search;
mod search;
mod servers;
mod session;
mod settings;
mod settings_panel;
mod terminal;
mod text_input;
mod theme;
mod tools;
mod ui;
mod welcome;
mod workspace;
mod wrap;

use gpui::{
    App, Application, Bounds, Focusable, TitlebarOptions, WindowBounds, WindowOptions, point, prelude::*, px, size,
};
use menus::Quit;
use std::path::{Path, PathBuf};
use workspace::Workspace;

/// `null` opens the current folder, `null <folder>` opens that folder, and
/// `null <file>` opens the file inside the current folder (or its own folder
/// when it lives elsewhere).
fn resolve_args() -> (PathBuf, Option<PathBuf>) {
    let cwd = std::env::current_dir().map(|d| absolute(&d)).unwrap_or_else(|_| PathBuf::from("."));
    let Some(arg) = std::env::args().nth(1) else {
        // Opened from the Finder (or the Dock), there's no folder to go by: the last project.
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let from_finder = cwd == Path::new("/") || Some(&cwd) == home.as_ref();
        if from_finder && let Some(last) = session::last_project() {
            return (last, None);
        }
        return (cwd, None);
    };
    // `null .` and `null` are the same project, with the same session: no `/.` at the end.
    let path = absolute(&cwd.join(arg));
    if path.is_dir() {
        return (path, None);
    }
    let root = if path.starts_with(&cwd) { cwd } else { path.parent().map(PathBuf::from).unwrap_or(cwd) };
    (root, Some(path))
}

/// The path with `.`, `..` and links resolved. A file that doesn't exist yet (`null new.rs`)
/// keeps its name, under its resolved folder.
fn absolute(path: &Path) -> PathBuf {
    if let Ok(path) = std::fs::canonicalize(path) {
        return path;
    }
    match (path.parent().and_then(|p| std::fs::canonicalize(p).ok()), path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
}

/// The window where it was last time, if that's still on a screen; centred otherwise.
fn window_bounds(saved: Option<session::WindowState>, cx: &App) -> WindowBounds {
    let centered = || Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let Some(saved) = saved else { return WindowBounds::Windowed(centered()) };
    let bounds =
        Bounds::new(point(px(saved.x), px(saved.y)), size(px(saved.width.max(480.)), px(saved.height.max(320.))));
    // At least its title bar must be on some display, or it would open out of reach.
    let title_bar = Bounds::new(bounds.origin, size(bounds.size.width, px(40.)));
    let visible = cx.displays().iter().any(|d| d.bounds().intersects(&title_bar));
    match (visible, saved.maximized) {
        (false, _) => WindowBounds::Windowed(centered()),
        (true, true) => WindowBounds::Maximized(bounds),
        (true, false) => WindowBounds::Windowed(bounds),
    }
}

fn main() {
    let (root, file) = resolve_args();

    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        settings::init(cx);
        keymap::register(cx.global::<settings::Settings>().keymap, cx);
        // ⌘Q with no window focused still asks about unsaved changes, in the main window.
        cx.on_action(|_: &Quit, cx| {
            let workspace = cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>());
            match workspace {
                Some(handle) => {
                    handle.update(cx, |workspace, window, cx| workspace.quit(&Quit, window, cx)).ok();
                }
                None => cx.quit(),
            }
        });
        menus::init(cx);
        menus::set(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let session = session::Session::load(&root);
        let options = WindowOptions {
            window_bounds: Some(window_bounds(session.window, cx)),
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
                window.focus(&workspace.focus_handle(cx));
                workspace.restore_session(session, window, cx);
                if let Some(file) = file {
                    workspace.open_file(file, window, cx);
                }
                if !cx.global::<settings::Settings>().welcomed {
                    workspace.show_welcome(window, cx);
                }
                workspace
            })
        })
        .expect("failed to open the main window");
        cx.activate(true);
    });
}
