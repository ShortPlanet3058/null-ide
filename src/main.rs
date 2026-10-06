mod ai;
mod ai_agent;
mod ai_task;
mod assets;
mod buffer;
mod dap;
mod debugger;
mod editor;
mod element;
mod encoding;
mod file_style;
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
mod local_history;
mod lsp;
mod lsp_store;
mod markdown;
mod markdown_view;
mod menus;
mod palette;
mod preview;
mod project_index;
mod project_search;
mod search;
mod servers;
mod session;
mod settings;
mod settings_panel;
mod tasks;
mod terminal;
mod terminal_links;
mod test_at;
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

/// A `file://` URL as a path ("%20" back to a space, and so on).
fn path_from_url(url: &str) -> Option<PathBuf> {
    let encoded = url.strip_prefix("file://")?;
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = encoded.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            decoded.push(byte);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8(decoded).ok()?))
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

    // Files and folders handed over by the Finder ("Open With", a drop on the Dock icon)
    // or by the `null` command while Null runs: opened in the window.
    let app = Application::new().with_assets(assets::Assets);
    let (opened, mut to_open) = futures::channel::mpsc::unbounded::<Vec<String>>();
    app.on_open_urls(move |urls| {
        opened.unbounded_send(urls).ok();
    });
    app.run(move |cx: &mut App| {
        cx.spawn(async move |cx| {
            use futures::StreamExt;
            while let Some(urls) = to_open.next().await {
                let paths: Vec<PathBuf> = urls.iter().filter_map(|u| path_from_url(u)).collect();
                cx.update(|cx| {
                    let workspace = cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>());
                    if let Some(handle) = workspace {
                        handle
                            .update(cx, |workspace, window, cx| {
                                window.activate_window();
                                workspace.open_paths(paths, window, cx)
                            })
                            .ok();
                    }
                })
                .ok();
            }
        })
        .detach();
        settings::init(cx);
        find_bar::init(cx);
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

        open_project_window(root, file, cx);
        cx.activate(true);
    });
}

/// The window already on project `root`, other than `except`. One project has one window:
/// two would keep (and clear) the same backup of unsaved work.
pub(crate) fn window_on(
    root: &std::path::Path,
    except: Option<gpui::AnyWindowHandle>,
    cx: &gpui::App,
) -> Option<gpui::WindowHandle<Workspace>> {
    let real = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let root = real(root);
    cx.windows()
        .into_iter()
        .filter(|w| Some(*w) != except)
        .filter_map(|w| w.downcast::<Workspace>())
        .find(|handle| handle.read(cx).is_ok_and(|workspace| real(workspace.root(cx)) == root))
}

/// Opens a window on project `root`, as it was left (and `file` in it, if given); the
/// window already on it, if there's one.
pub(crate) fn open_project_window(root: PathBuf, file: Option<PathBuf>, cx: &mut gpui::App) {
    if let Some(handle) = window_on(&root, None, cx) {
        handle
            .update(cx, |workspace, window, cx| {
                window.activate_window();
                if let Some(file) = file {
                    workspace.open_file(file, window, cx);
                }
            })
            .ok();
        return;
    }
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
    let opened = cx.open_window(options, |window, cx| {
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
    });
    if let Err(error) = opened {
        eprintln!("null: couldn't open a window: {error}");
    }
}

/// Quitting: the next window with unsaved changes asks about them; once none is left,
/// Null quits.
pub(crate) fn quit_next(cx: &mut gpui::App) {
    let waiting = cx
        .windows()
        .into_iter()
        .filter_map(|w| w.downcast::<Workspace>())
        .find(|handle| handle.read(cx).is_ok_and(|workspace| !workspace.quitting && workspace.has_unsaved(cx)));
    match waiting {
        Some(handle) => {
            handle
                .update(cx, |workspace, window, cx| {
                    window.activate_window();
                    workspace.quit(&Quit, window, cx)
                })
                .ok();
        }
        None => cx.quit(),
    }
}

#[cfg(test)]
mod url_tests {
    use super::*;

    #[test]
    fn urls_from_the_finder_become_paths() {
        assert_eq!(
            path_from_url("file:///Users/me/My%20Project/a.rs"),
            Some(PathBuf::from("/Users/me/My Project/a.rs"))
        );
        assert_eq!(path_from_url("file:///tmp/caf%C3%A9.txt"), Some(PathBuf::from("/tmp/café.txt")));
        assert_eq!(path_from_url("https://example.com"), None);
    }
}
