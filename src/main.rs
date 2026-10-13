// Ranges of text are everywhere here: a list of one range is a list, not a mistake.
#![allow(clippy::single_range_in_vec_init)]
// A window app on Windows: no console window opening beside it (debug builds keep one,
// for what they print).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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
mod instance;
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
mod roots;
mod search;
mod server_log;
mod servers;
mod session;
mod settings;
mod settings_panel;
mod shell;
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
mod window_frame;
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
    /// More folders, opened in the same window beside `root` (`null api web`).
    others: Vec<PathBuf>,
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
    // Several folders (`null api web`): one window with them all.
    if !wait && args.len() > 1 {
        let cwd = std::env::current_dir().map(|d| absolute(&d)).unwrap_or_else(|_| PathBuf::from("."));
        let folders: Vec<PathBuf> = args.iter().map(|a| absolute(&cwd.join(a))).collect();
        if folders.iter().all(|f| f.is_dir()) {
            let mut folders = folders.into_iter();
            let root = folders.next().unwrap_or(cwd);
            return Opening { root, others: folders.collect(), file: None, place: None, wait: false };
        }
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
    Opening { root, others: Vec::new(), file, place, wait }
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
        // (From Windows' Start menu or Explorer the folder is Null's own, or System32.)
        let home = crate::tools::home_dir().map(|h| absolute(&h));
        let own = std::env::current_exe().ok().and_then(|e| e.parent().map(absolute));
        let system = std::env::var_os("SystemRoot").map(|r| absolute(&PathBuf::from(r)));
        // (On Windows, null.exe started with nothing is a shortcut's or Explorer's, whatever
        // folder it starts in: `null` from a terminal passes the folder it's in.)
        let from_finder = cfg!(windows)
            || cwd.parent().is_none()
            || Some(&cwd) == home.as_ref()
            || Some(&cwd) == own.as_ref()
            || system.is_some_and(|r| cwd.starts_with(r));
        if from_finder {
            if let Some(last) = session::last_project() {
                return (last, None);
            }
            // The very first time: not the whole disk (or home) as a project, read and
            // watched, but an empty folder of Null's own until one is opened.
            if let Some(scratch) = session::scratch_folder() {
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
    if let Ok(path) = dunce::canonicalize(path) {
        return path;
    }
    match (path.parent().and_then(|p| dunce::canonicalize(p).ok()), path.file_name()) {
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

/// What `null --help` says.
const HELP: &str = "Null, a calm, fast code editor.

  null                  the last project (from a terminal: the folder you're in)
  null PATH             a folder as the project, or a file in it
  null FILE:LINE[:COL]  a file at a line (and column)
  null A B ...          several folders in one window
  null --wait FILE      returns once the file's tab is closed (git's editor)
  null --version        Null's version
";

/// `--version` and `--help`, answered in the terminal (Null not started); and on Linux, no
/// display to open a window on said, not waited on forever.
fn answered_in_the_terminal() -> bool {
    let first = std::env::args().nth(1);
    let answer = match first.as_deref() {
        Some("--version" | "-V") => format!("Null {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h") => HELP.trim_end().to_string(),
        _ => {
            if cfg!(all(unix, not(target_os = "macos")))
                && std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty())
                && std::env::var_os("WAYLAND_DISPLAY").is_none_or(|d| d.is_empty())
            {
                eprintln!("Null needs a graphical session: neither DISPLAY nor WAYLAND_DISPLAY is set.");
                std::process::exit(1);
            }
            return false;
        }
    };
    // (On Windows Null has no console of its own: the one it was started from, if any.)
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn AttachConsole(process: u32) -> i32;
        }
        // SAFETY: a plain Win32 call with no pointers; it fails harmlessly with no console.
        unsafe { AttachConsole(u32::MAX) };
    }
    println!("{answer}");
    true
}

fn main() {
    if answered_in_the_terminal() {
        return;
    }
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
    // Started by the Mac with nothing to open (the Dock, the Finder, or `null a b` through
    // `open -a`, whose paths come a moment later as "open these").
    let by_the_system = cfg!(target_os = "macos") && std::env::args().len() <= 1;
    let Opening { root, others, file, place, wait } = resolve_args();
    trace::mark("what to open worked out");
    // Off the Mac: a Null already running opens it instead (a waiting one is its own).
    let request = instance::Request { root: root.clone(), others: others.clone(), file: file.clone(), place };
    let mut handed = match instance::launch(&request, wait) {
        instance::Launch::HandedOver => return,
        instance::Launch::First(handed) => handed,
    };

    // Files and folders handed over by the Finder ("Open With", a drop on the Dock icon)
    // or by the `null` command while Null runs: opened in the window.
    let app = Application::new().with_assets(assets::Assets);
    trace::mark("app made");
    let (opened, mut to_open) = futures::channel::mpsc::unbounded::<Vec<String>>();
    let handed_paths = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let saw_paths = handed_paths.clone();
    app.on_open_urls(move |urls| {
        saw_paths.store(true, std::sync::atomic::Ordering::SeqCst);
        opened.unbounded_send(urls).ok();
    });
    app.run(move |cx: &mut App| {
        trace::mark("app running");
        cx.spawn(async move |cx| {
            use futures::StreamExt;
            while let Some(urls) = to_open.next().await {
                let paths: Vec<PathBuf> = urls.iter().filter_map(|u| path_from_url(u)).collect();
                // Several folders at once (`null api web`, or dropped together on the Dock
                // icon): one window with them all.
                if paths.len() > 1 && paths.iter().all(|p| p.is_dir()) {
                    let mut folders = paths.into_iter();
                    let root = folders.next().unwrap_or_default();
                    cx.update(|cx| {
                        open_project_window_now(root.clone(), None, None, cx);
                        add_folders(&root, folders.collect(), cx);
                    })
                    .ok();
                    continue;
                }
                cx.update(|cx| {
                    let workspace = cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>());
                    // Null was started for these: the first is the window's project (a
                    // file's folder, with the file open), the rest open in it.
                    if workspace.is_none() {
                        let mut paths = paths.into_iter();
                        let Some(first) = paths.next() else { return };
                        let (root, file) = if first.is_dir() {
                            (first, None)
                        } else {
                            (first.parent().map(PathBuf::from).unwrap_or_default(), Some(first))
                        };
                        open_project_window_now(root.clone(), file, None, cx);
                        let rest: Vec<PathBuf> = paths.collect();
                        if !rest.is_empty()
                            && let Some(handle) = window_on(&root, None, cx)
                        {
                            handle.update(cx, |workspace, window, cx| workspace.open_paths(rest, window, cx)).ok();
                        }
                        return;
                    }
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
        // What later launches hand over (off the Mac): opened as they would have opened.
        if let Some(mut handed) = handed.take() {
            cx.spawn(async move |cx| {
                use futures::StreamExt;
                while let Some(request) = handed.next().await {
                    cx.update(|cx| {
                        // A file in a project a window has open: opened there.
                        if request.others.is_empty()
                            && let Some(file) = &request.file
                            && let Some(handle) = window_holding(file, cx)
                        {
                            let (file, place) = (file.clone(), request.place);
                            handle
                                .update(cx, |workspace, window, cx| {
                                    window.activate_window();
                                    open_at(workspace, file, place, window, cx);
                                })
                                .ok();
                            return;
                        }
                        let root = request.root.clone();
                        open_project_window_now(request.root, request.file, request.place, cx);
                        add_folders(&root, request.others, cx);
                    })
                    .ok();
                }
            })
            .detach();
        }
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
            instance::forget();
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
        } else if by_the_system && !handed_paths.load(std::sync::atomic::Ordering::SeqCst) {
            // Paths may be on their way: then they're what opens, not the last project.
            // (A timer, not a deferred open: that would wait for an update that never comes.)
            let handed_paths = handed_paths.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(350)).await;
                cx.update(|cx| {
                    if !handed_paths.load(std::sync::atomic::Ordering::SeqCst) && cx.windows().is_empty() {
                        open_project_window_now(root, file, place, cx);
                    }
                })
                .ok();
            })
            .detach();
        } else if by_the_system {
            // (Already here: the paths open the window.)
        } else {
            let first = root.clone();
            open_project_window_now(root, file, place, cx);
            add_folders(&first, others, cx);
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

/// The window already on project `root` (one of its folders), other than `except`. One
/// project has one window: two would keep (and clear) the same backup of unsaved work.
pub(crate) fn window_on(
    root: &std::path::Path,
    except: Option<gpui::AnyWindowHandle>,
    cx: &gpui::App,
) -> Option<gpui::WindowHandle<Workspace>> {
    let real = |p: &std::path::Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let root = real(root);
    cx.windows()
        .into_iter()
        .filter(|w| Some(*w) != except)
        .filter_map(|w| w.downcast::<Workspace>())
        .find(|handle| handle.read(cx).is_ok_and(|workspace| workspace.roots().all().iter().any(|r| real(r) == root)))
}

/// `others` added to the window on `root` (just opened), beside it.
fn add_folders(root: &std::path::Path, others: Vec<PathBuf>, cx: &mut gpui::App) {
    if others.is_empty() {
        return;
    }
    if let Some(handle) = window_on(root, None, cx) {
        handle.update(cx, |workspace, window, cx| workspace.add_roots(others, window, cx)).ok();
    }
}

/// The window with a folder holding `file` (the innermost, should several), if one does.
fn window_holding(file: &std::path::Path, cx: &gpui::App) -> Option<gpui::WindowHandle<Workspace>> {
    let real = |p: &std::path::Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let file = real(file);
    cx.windows()
        .into_iter()
        .filter_map(|w| w.downcast::<Workspace>())
        .filter_map(|handle| {
            // Any of its folders; the innermost that holds the file counts.
            let workspace = handle.read(cx).ok()?;
            let depth = workspace
                .roots()
                .all()
                .iter()
                .map(|r| real(r))
                .filter(|r| file.starts_with(r))
                .map(|r| r.as_os_str().len())
                .max()?;
            Some((depth, handle))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, handle)| handle)
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

/// What every window shares off the Mac: on Linux, its frame drawn by Null (as on the Mac
/// and Windows, the title bar is Null's) and a name for the desktop to know its icon by.
fn frame() -> WindowOptions {
    WindowOptions {
        app_id: Some("null".into()),
        window_decorations: cfg!(target_os = "linux").then_some(gpui::WindowDecorations::Client),
        ..Default::default()
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
        ..frame()
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
        ..frame()
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
