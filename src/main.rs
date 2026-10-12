// Ranges of text are everywhere here: a list of one range is a list, not a mistake.
#![allow(clippy::single_range_in_vec_init)]

mod ai;
mod ai_agent;
mod ai_task;
mod assets;
mod basic_syntax;
mod buffer;
mod clipboard_history;
mod color_panel;
mod colors;
mod commit_page;
mod crash;
mod dap;
mod debugger;
mod editor;
mod element;
mod emmet;
mod encoding;
mod file_style;
mod file_tree;
mod find_bar;
mod fonts;
mod fs_ops;
mod fuzzy;
mod git;
mod git_view;
mod github;
mod github_page;
mod highlight;
mod html_markdown;
mod json_format;
mod key_prompt;
mod keymap;
mod languages;
mod launch;
mod local_history;
mod lsp;
mod lsp_store;
mod markdown;
mod markdown_html;
mod markdown_links;
mod markdown_view;
mod math;
mod menus;
mod notebook;
mod ollama;
mod outline;
mod palette;
mod preview;
mod problem_matcher;
mod project_index;
mod project_search;
mod punctuation;
mod python_env;
#[cfg(debug_assertions)]
mod qa;
mod search;
mod server_log;
mod servers;
mod session;
mod settings;
mod settings_panel;
mod snippets;
mod spell;
mod svg_image;
mod system_clipboard;
mod tasks;
mod terminal;
mod terminal_links;
mod terminal_watch;
mod test_at;
mod text_input;
mod theme;
mod tools;
mod trace;
mod ui;
mod user_keys;
mod vscode_import;
mod welcome;
mod word_blocks;
mod workspace;
mod wrap;

use gpui::{
    App, Application, Bounds, Focusable, TitlebarOptions, WindowBounds, WindowOptions, point, prelude::*, px, size,
};
use menus::Quit;
use std::path::{Path, PathBuf};
use workspace::Workspace;

/// What Null was started with: the project, a file in it (at a line, `a.rs:42:7`), and
/// whether the one who started it waits for that file's tab to close (`--wait`, as git's
/// editor).
struct Opening {
    root: PathBuf,
    file: Option<PathBuf>,
    place: Option<(u32, u32)>,
    wait: bool,
}

/// `a.rs:42` or `a.rs:42:7`, as compilers write places: the path, and the line and column
/// (from 1). Anything else: the path as it is.
fn split_place(arg: &str) -> (&str, Option<(u32, u32)>) {
    let number =
        |s: &str| (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse::<u32>().ok()).flatten();
    let mut parts = arg.rsplitn(3, ':');
    let (last, middle, first) = (parts.next(), parts.next(), parts.next());
    match (first, middle.and_then(number), last.and_then(number)) {
        (Some(path), Some(line), Some(column)) if !path.is_empty() => (path, Some((line, column))),
        _ => match arg.rsplit_once(':') {
            Some((path, line)) if !path.is_empty() => match number(line) {
                Some(line) => (path, Some((line, 1))),
                None => (arg, None),
            },
            _ => (arg, None),
        },
    }
}

fn resolve_args() -> Opening {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let wait = args.first().is_some_and(|a| a == "--wait");
    if wait {
        args.remove(0);
    }
    let (root, file) = resolve_path(args.first().cloned());
    // A file named with its place (`a.rs:42`) when there's no file by that whole name.
    let (file, place) = match file {
        Some(file) if !file.exists() => {
            let name = file.to_string_lossy().into_owned();
            match split_place(&name) {
                (path, Some(place)) if std::path::Path::new(path).is_file() => (Some(PathBuf::from(path)), Some(place)),
                _ => (Some(file), None),
            }
        }
        file => (file, None),
    };
    // `--wait` is for a file (git's message); its project, the repository it's in (or its
    // folder): not where the one asking happens to be (an app's is `/`).
    let wait = wait && file.is_some();
    let root = match (&file, wait) {
        (Some(file), true) => repository_of(file),
        _ => root,
    };
    Opening { root, file, place, wait }
}

/// The repository `file` is in (the folder holding `.git`), else its own folder.
fn repository_of(file: &std::path::Path) -> PathBuf {
    let folder = file.parent().unwrap_or(file);
    folder
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(|dir| if dir.ends_with(".git") { dir.parent().unwrap_or(dir) } else { dir })
        .unwrap_or(folder)
        .to_path_buf()
}

/// `null` opens the current folder, `null <folder>` opens that folder, and
/// `null <file>` opens the file inside the current folder (or its own folder
/// when it lives elsewhere).
fn resolve_path(arg: Option<String>) -> (PathBuf, Option<PathBuf>) {
    let cwd = std::env::current_dir().map(|d| absolute(&d)).unwrap_or_else(|_| PathBuf::from("."));
    let Some(arg) = arg else {
        // Opened from the Finder (or the Dock), there's no folder to go by: the last project.
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let from_finder = cwd == Path::new("/") || Some(&cwd) == home.as_ref();
        if from_finder {
            if let Some(last) = session::last_project() {
                return (last, None);
            }
            // The very first time: not the whole disk (or home) as a project, read and
            // watched, but an empty folder of Null's own until one is opened.
            if let Some(scratch) = tools::data_dir().map(|d| d.join("Scratch")) {
                std::fs::create_dir_all(&scratch).ok();
                return (scratch, None);
            }
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
    trace::start();
    trace::mark("main");
    crash::record();
    // The colouring rules of the usual languages, made ready meanwhile (each took 25 ms of
    // opening its first file).
    std::thread::spawn(|| {
        for name in ["Rust", "Python", "TypeScript", "TSX", "JavaScript", "JSON", "Markdown", "C", "C++", "Go"] {
            if let Some(language) = languages::all().iter().find(|l| l.name == name) {
                language.highlight_query();
            }
        }
    });
    let Opening { root, file, place, wait } = resolve_args();
    trace::mark("what to open worked out");

    // Files and folders handed over by the Finder ("Open With", a drop on the Dock icon)
    // or by the `null` command while Null runs: opened in the window.
    let app = Application::new().with_assets(assets::Assets);
    trace::mark("app made");
    let (opened, mut to_open) = futures::channel::mpsc::unbounded::<Vec<String>>();
    app.on_open_urls(move |urls| {
        opened.unbounded_send(urls).ok();
    });
    app.run(move |cx: &mut App| {
        trace::mark("app running");
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
        // Local history past its month goes, now and then.
        std::thread::spawn(local_history::sweep);
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
        // The Ollama Null started (see `ollama`) goes with it.
        cx.on_app_quit(|_| {
            ollama::stop();
            async {}
        })
        .detach();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        // Nothing else is open yet: the window opens now (a deferred open would wait for
        // an update that never comes, and Null would start with no window).
        trace::mark("settings, keys and menus ready");
        if wait {
            open_waiting_window(root, file, place, cx);
        } else {
            open_project_window_now(root, file, place, cx);
        }
        trace::mark("window opened");
        // The spell checker started once the window's up, then what's shown checked.
        cx.spawn(async move |cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
            cx.update(|cx| {
                let _step = trace::step(|| "spell checker started".into());
                spell::warm();
                cx.refresh_windows();
            })
            .ok();
        })
        .detach();
        #[cfg(debug_assertions)]
        qa::run(cx);
        // (Looked at by `qa` while someone works in another app: not brought to the front
        // unless asked.)
        let behind = cfg!(debug_assertions)
            && std::env::var_os("NULL_QA").is_some()
            && std::env::var_os("NULL_QA_FRONT").is_none_or(|front| front == "0");
        if !behind {
            cx.activate(true);
        }
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
    // Once the window asking (if any) is done updating: until then it can't be read, so
    // its own project wouldn't be found open.
    cx.defer(move |cx| open_project_window_now(root, file, None, cx));
}

/// `file` opened in `workspace`, at `place` (line and column, from 1) when given.
fn open_at(
    workspace: &mut Workspace,
    file: PathBuf,
    place: Option<(u32, u32)>,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) {
    match place {
        Some((line, column)) => {
            let at = lsp_types::Position::new(line.saturating_sub(1), column.saturating_sub(1));
            workspace.go_to(file, lsp_types::Range::new(at, at), window, cx);
        }
        None => workspace.open_file(file, window, cx),
    }
}

/// `null --wait <file>` (git's editor): a window with that file alone, nothing of the
/// project's session read or written; closing its tab (or the window) quits, and the one
/// waiting goes on.
fn open_waiting_window(root: PathBuf, file: Option<PathBuf>, place: Option<(u32, u32)>, cx: &mut gpui::App) {
    let options = WindowOptions {
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
            if let Some(file) = file {
                workspace.waiting_for = Some(file.clone());
                open_at(&mut workspace, file, place, window, cx);
            }
            workspace
        })
    });
    if opened.is_err() {
        cx.quit();
    }
}

fn open_project_window_now(root: PathBuf, file: Option<PathBuf>, place: Option<(u32, u32)>, cx: &mut gpui::App) {
    if let Some(handle) = window_on(&root, None, cx) {
        handle
            .update(cx, |workspace, window, cx| {
                window.activate_window();
                if let Some(file) = file {
                    open_at(workspace, file, place, window, cx);
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
                open_at(&mut workspace, file, place, window, cx);
            }
            if !cx.global::<settings::Settings>().welcomed {
                workspace.show_welcome(window, cx);
            }
            workspace
        })
    });
    if let Err(error) = opened {
        eprintln!("null: couldn't open a window: {error}");
        // Nothing open, nothing would ever close: don't stay running out of sight.
        if cx.windows().is_empty() {
            cx.quit();
        }
    }
}

/// Quitting: the next window with unsaved changes (or a command running in a terminal)
/// asks about them; once none is left, Null quits.
/// Quitting was called off in one window: none of them counts as done asking any more.
pub(crate) fn quit_cancelled(cx: &mut gpui::App) {
    for handle in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
        handle.update(cx, |workspace, _, _| workspace.quitting = false).ok();
    }
}

pub(crate) fn quit_next(cx: &mut gpui::App) {
    let waiting = cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()).find(|handle| {
        handle.read(cx).is_ok_and(|workspace| {
            !workspace.quitting && (workspace.has_unsaved(cx) || workspace.commands_running(cx).is_some())
        })
    });
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
    fn a_waited_file_s_project_is_its_repository() {
        let dir = crate::tools::test_dir("wait-root");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        assert_eq!(repository_of(&dir.join(".git/COMMIT_EDITMSG")), dir);
        assert_eq!(repository_of(&dir.join("docs/a.md")), dir);
        let alone = crate::tools::test_dir("wait-alone");
        std::fs::create_dir_all(&alone).unwrap();
        assert_eq!(repository_of(&alone.join("x.txt")), alone);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&alone).ok();
    }

    #[test]
    fn places_after_a_file_s_name() {
        assert_eq!(split_place("src/a.rs:42:7"), ("src/a.rs", Some((42, 7))));
        assert_eq!(split_place("a.rs:42"), ("a.rs", Some((42, 1))));
        assert_eq!(split_place("a.rs"), ("a.rs", None));
        assert_eq!(split_place("C:x"), ("C:x", None));
        assert_eq!(split_place("notes:draft.md"), ("notes:draft.md", None));
    }

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
