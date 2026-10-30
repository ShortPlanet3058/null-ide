// Ranges of text are everywhere here: a list of one range is a list, not a mistake.
#![allow(clippy::single_range_in_vec_init)]
// A window app on Windows: no console window opening beside it (debug builds keep one,
// for what they print).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ai;
mod ai_agent;
mod ai_context;
mod ai_task;
mod ai_usage;
mod askpass;
mod assets;
mod basic_syntax;
mod buffer;
mod clipboard_history;
mod code_notes;
mod color_panel;
mod colors;
mod commit_page;
mod crash;
mod dap;
mod debugger;
mod download;
mod drawing;
mod editor;
mod element;
mod emmet;
mod encoding;
mod file_icons;
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
mod modal;
mod node_runtime;
mod notebook;
mod ollama;
mod outline;
mod palette;
mod preview;
mod problem_matcher;
mod project_fs;
mod project_index;
mod project_search;
mod punctuation;
mod python_env;
#[cfg(debug_assertions)]
mod qa;
mod remote;
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
mod ssh_prompt;
mod svg_image;
mod system_clipboard;
mod tasks;
mod terminal;
mod terminal_ai;
mod terminal_links;
mod terminal_watch;
mod test_at;
mod text_input;
mod theme;
mod tools;
mod trace;
mod trash_prompt;
mod trust;
mod ui;
mod updates;
mod user_keys;
mod vscode_import;
mod welcome;
mod whats_new;
mod window_frame;
mod word_blocks;
mod workspace;
mod wrap;
mod wsl;

use gpui::{App, Bounds, Focusable, TitlebarOptions, WindowBounds, WindowOptions, point, prelude::*, px, size};
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
    /// More files after it (`null a.rs b.rs`), each at its place.
    more: Vec<(PathBuf, Option<(u32, u32)>)>,
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

/// What `null` was asked, its options anywhere before a `--`: `--wait`, and the paths.
#[derive(Debug, Default, PartialEq)]
struct Asked {
    wait: bool,
    help: bool,
    version: bool,
    /// As the system gave them (a name that isn't UTF-8 kept as it is: it's the file's).
    paths: Vec<std::ffi::OsString>,
}

/// The arguments read (Err: an option Null doesn't have, said as the terminal shows it).
/// After `--`, everything is a path (a file named `-x`).
/// The command line's arguments (not the program's name), as the system gives them.
/// (`std::env::args` panics on one that isn't UTF-8: a Latin-1 file name opened from the
/// desktop's files kept Null from starting at all.)
fn arguments() -> Vec<std::ffi::OsString> {
    std::env::args_os().skip(1).collect()
}

/// The arguments as text, a name that isn't UTF-8 with its odd bytes replaced (for what
/// reads them as words: the updater's).
fn arguments_text() -> Vec<String> {
    arguments().into_iter().map(|a| a.to_string_lossy().into_owned()).collect()
}

fn parse_args(args: &[std::ffi::OsString]) -> Result<Asked, String> {
    let mut asked = Asked::default();
    let mut options = true;
    for arg in args {
        // (A name that isn't UTF-8 isn't an option: a path.)
        let Some(text) = arg.to_str() else {
            asked.paths.push(arg.clone());
            continue;
        };
        match text {
            "--" if options => options = false,
            "--wait" | "-w" if options => asked.wait = true,
            "--help" | "-h" if options => asked.help = true,
            "--version" | "-V" if options => asked.version = true,
            // (What the Mac's Finder once passed: not an option of ours.)
            other if options && other.starts_with("-psn_") => {}
            other if options && other.starts_with('-') && other.len() > 1 => {
                return Err(format!(
                    "null: unknown option {other} (null --help lists them; -- before a file named so)"
                ));
            }
            _ => asked.paths.push(arg.clone()),
        }
    }
    Ok(asked)
}

/// `path` with the place written after its name (`a.rs:42`) when there's no file by that
/// whole name.
fn with_place(path: PathBuf) -> (PathBuf, Option<(u32, u32)>) {
    if path.exists() {
        return (path, None);
    }
    let name = path.to_string_lossy().into_owned();
    match split_place(&name) {
        (file, Some(place)) if Path::new(file).is_file() => (PathBuf::from(file), Some(place)),
        _ => (path, None),
    }
}

fn resolve_args() -> Opening {
    let args = arguments();
    // (An unknown option was said and Null didn't start: see `answered_in_the_terminal`.)
    let Asked { wait, paths, .. } = parse_args(&args).unwrap_or_default();
    // `--wait` is for a file (git's message); its project, the repository it's in (or its
    // folder): not where the one asking happens to be (an app's is `/`).
    if wait {
        let (root, file) = resolve_path(paths.first().cloned());
        let (file, place) = match file.map(with_place) {
            Some((file, place)) => (Some(file), place),
            None => (None, None),
        };
        let root = file.as_deref().map_or(root, repository_of);
        let wait = file.is_some();
        return Opening { root, others: Vec::new(), file, place, more: Vec::new(), wait };
    }
    if paths.len() <= 1 {
        let (root, file) = resolve_path(paths.first().cloned());
        let (file, place) = match file.map(with_place) {
            Some((file, place)) => (Some(file), place),
            None => (None, None),
        };
        return Opening { root, others: Vec::new(), file, place, more: Vec::new(), wait: false };
    }
    // Several (`null api web`, `null a.rs b.rs`, `null . a.rs`): the folders in one window,
    // the files opened in it; with no folder, the first file's project.
    let cwd = std::env::current_dir().map(|d| absolute(&d)).unwrap_or_else(|_| PathBuf::from("."));
    let (mut folders, mut files) = (Vec::new(), Vec::new());
    for arg in &paths {
        match arg.to_str().and_then(server_folder) {
            Some(server) => folders.push(server),
            None => {
                let path = absolute(&cwd.join(arg));
                if path.is_dir() { folders.push(path) } else { files.push(with_place(path)) }
            }
        }
    }
    let mut files = files.into_iter();
    let first = files.next();
    let root = match (folders.first(), &first) {
        (Some(folder), _) => folder.clone(),
        (None, Some((file, _))) => root_for_file(&cwd, file),
        (None, None) => cwd,
    };
    let others = folders.into_iter().skip(1).collect();
    let (file, place) = match first {
        Some((file, place)) => (Some(file), place),
        None => (None, None),
    };
    Opening { root, others, file, place, more: files.collect(), wait: false }
}

/// The project for `file` given from a terminal in `cwd`: that folder, when the file's in
/// it, unless it's too wide to be one (the home folder, the disk) or above the repository
/// the file's in (`~/code` for `~/code/app/src/a.rs`): then that repository (or, with
/// none, the file's own folder).
fn root_for_file(cwd: &Path, file: &Path) -> PathBuf {
    let home = crate::tools::home_dir().map(|h| absolute(&h));
    let too_wide = cwd.parent().is_none() || Some(cwd) == home.as_deref();
    let repository = repository_holding(file);
    let above_it = repository.as_ref().is_some_and(|r| r != cwd && r.starts_with(cwd));
    if file.starts_with(cwd) && !too_wide && !above_it {
        return cwd.to_path_buf();
    }
    repository.unwrap_or_else(|| file.parent().map_or_else(|| cwd.to_path_buf(), Path::to_path_buf))
}

/// The repository `file` is in (the folder holding `.git`), if any.
fn repository_holding(file: &Path) -> Option<PathBuf> {
    let folder = file.parent().unwrap_or(file);
    folder
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(|dir| if dir.ends_with(".git") { dir.parent().unwrap_or(dir) } else { dir }.to_path_buf())
}

/// The repository `file` is in (the folder holding `.git`), else its own folder.
fn repository_of(file: &std::path::Path) -> PathBuf {
    repository_holding(file).unwrap_or_else(|| file.parent().unwrap_or(file).to_path_buf())
}

/// `null` opens the current folder, `null <folder>` opens that folder, and
/// `null <file>` opens the file inside the current folder (or its own folder
/// when it lives elsewhere).
fn resolve_path(arg: Option<std::ffi::OsString>) -> (PathBuf, Option<PathBuf>) {
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
    // A folder on a server: `ssh://me@box:2222/home/me/app`, or as Null writes it
    // (`/ssh:me@box:/home/me/app`).
    if let Some(server) = arg.to_str().and_then(server_folder) {
        return (server, None);
    }
    // `null .` and `null` are the same project, with the same session: no `/.` at the end.
    let path = absolute(&cwd.join(arg));
    if path.is_dir() {
        return (path, None);
    }
    (root_for_file(&cwd, &path), Some(path))
}

/// A folder on a server, as `null` is given it: `ssh://[user@]host[:port]/path` (a path
/// from the home folder with `/~/`), or `/ssh:host:/path`.
fn server_folder(arg: &str) -> Option<PathBuf> {
    if remote::is_remote(Path::new(arg)) {
        return Some(PathBuf::from(arg));
    }
    let rest = arg.strip_prefix("ssh://")?;
    let (address, path) = rest.split_once('/').unwrap_or((rest, ""));
    // (A port after the last colon, unless it's inside an address in brackets.)
    let host = match address.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) && !port.contains(']') => {
            format!("{host}#{port}")
        }
        _ => address.to_string(),
    };
    if host.is_empty() {
        return None;
    }
    let path = match path.strip_prefix('~') {
        Some(rest) => format!("{}{rest}", remote::home(&host).ok()?),
        None => format!("/{path}"),
    };
    Some(remote::path(&host, &path))
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
        return wsl::plain(path);
    }
    match (path.parent().and_then(|p| dunce::canonicalize(p).ok()), path.file_name()) {
        (Some(parent), Some(name)) => wsl::plain(parent.join(name)),
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
  null A B ...          several folders in one window, files opened in it
  null --wait FILE      returns once the file's tab is closed (git's editor; -w)
  null --version        Null's version (-V)
  null --help           this (-h)
  null -- -FILE         a file whose name starts with -
";

/// `--version` and `--help`, answered in the terminal (Null not started); and on Linux, no
/// display to open a window on said, not waited on forever.
fn answered_in_the_terminal() -> bool {
    let first = arguments_text().into_iter().next();
    let asked = match parse_args(&arguments()) {
        Ok(asked) => asked,
        // (An uninstaller's own, first: below.)
        Err(_) if first.as_deref() == Some("--forget-keys") => Asked::default(),
        Err(unknown) => {
            eprintln!("{unknown}");
            std::process::exit(2);
        }
    };
    let answer = match first.as_deref() {
        _ if asked.version => format!("Null {}", env!("CARGO_PKG_VERSION")),
        _ if asked.help => HELP.trim_end().to_string(),
        // (For the uninstallers, when asked to delete everything: not in --help.)
        Some("--forget-keys") => match ai::forget_api_keys() {
            1 => "Deleted 1 saved API key.".to_string(),
            n => format!("Deleted {n} saved API keys."),
        },
        _ => {
            if cfg!(all(unix, not(target_os = "macos")))
                && std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty())
                && std::env::var_os("WAYLAND_DISPLAY").is_none_or(|d| d.is_empty())
            {
                eprintln!("Null needs a graphical session: neither DISPLAY nor WAYLAND_DISPLAY is set.");
                std::process::exit(1);
            }
            // Set, but to a display that's gone (an ssh session's DISPLAY=:99, a desktop
            // restarted): said, not a crash in GPUI (which would also leave a crash log).
            #[cfg(all(unix, not(target_os = "macos")))]
            if let Some(missing) = display_missing() {
                eprintln!("Null needs a graphical session: {missing}.");
                std::process::exit(1);
            }
            return false;
        }
    };
    // (On Windows Null has no console of its own: the one it was started from, if any. Joined,
    // a windowed program has no handle to write to it with (none, or cmd's own, not valid
    // here): the console's own output is opened then. Output sent to a file or a pipe is
    // left as it is.)
    #[cfg(windows)]
    {
        const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
        const GENERIC_WRITE: u32 = 0x4000_0000;
        const FILE_SHARE_WRITE: u32 = 2;
        const OPEN_EXISTING: u32 = 3;
        unsafe extern "system" {
            fn AttachConsole(process: u32) -> i32;
            fn GetStdHandle(which: u32) -> isize;
            fn GetFileType(handle: isize) -> u32;
            fn SetStdHandle(which: u32, handle: isize) -> i32;
            fn CreateFileW(
                name: *const u16,
                access: u32,
                share: u32,
                security: *const std::ffi::c_void,
                disposition: u32,
                flags: u32,
                template: isize,
            ) -> isize;
        }
        // SAFETY: plain Win32 calls; the only pointer is a NUL-terminated name that lives
        // through the call. Each fails harmlessly when there's no console.
        unsafe {
            const FILE_TYPE_DISK: u32 = 1;
            const FILE_TYPE_PIPE: u32 = 3;
            let had = GetStdHandle(STD_OUTPUT_HANDLE);
            let redirected = had != 0 && had != -1 && matches!(GetFileType(had), FILE_TYPE_DISK | FILE_TYPE_PIPE);
            if AttachConsole(u32::MAX) != 0 && !redirected {
                let name: Vec<u16> = "CONOUT$".encode_utf16().chain([0]).collect();
                let out =
                    CreateFileW(name.as_ptr(), GENERIC_WRITE, FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, 0);
                if out != -1 {
                    SetStdHandle(STD_OUTPUT_HANDLE, out);
                }
            }
        }
    }
    println!("{answer}");
    true
}

/// The display named isn't there: neither Wayland's socket nor (for a local X display,
/// `:0`) X's. A display elsewhere (`localhost:10.0`, ssh -X) is left to try.
#[cfg(all(unix, not(target_os = "macos")))]
fn display_missing() -> Option<String> {
    let set = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let wayland = set("WAYLAND_DISPLAY").map(|name| {
        let runtime = set("XDG_RUNTIME_DIR").unwrap_or_default();
        let socket = if name.starts_with('/') { PathBuf::from(&name) } else { Path::new(&runtime).join(&name) };
        (name, socket.exists())
    });
    let x = set("DISPLAY").map(|name| {
        let number = name.strip_prefix(':').map(|n| n.split('.').next().unwrap_or(n).to_string());
        let there = number.is_none_or(|n| Path::new(&format!("/tmp/.X11-unix/X{n}")).exists());
        (name, there)
    });
    match (wayland, x) {
        (Some((_, true)), _) | (_, Some((_, true))) => None,
        (Some((w, _)), Some((d, _))) => Some(format!("no display at WAYLAND_DISPLAY={w} or DISPLAY={d}")),
        (Some((w, _)), None) => Some(format!("no display at WAYLAND_DISPLAY={w}")),
        (None, Some((d, _))) => Some(format!("no display at DISPLAY={d}")),
        (None, None) => None,
    }
}

fn main() {
    // Started by ssh to ask something (see `askpass`): asked in the window of the Null that
    // started that ssh, answered, done.
    if let Some(code) = askpass::answered_for_ssh() {
        std::process::exit(code);
    }
    // Started by Null as it quit, to install an update (see `updates`).
    #[cfg(windows)]
    if arguments_text().first().map(String::as_str) == Some("--finish-update") {
        std::process::exit(updates::finish(&arguments_text()[1..]));
    }
    // (ssh started from here asks nothing in the window: see `remote::asking`.)
    remote::note_main_thread();
    if answered_in_the_terminal() {
        return;
    }
    trace::start();
    trace::mark("main");
    tools::keep_data_private();
    crash::record();
    trust::start();
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
    let by_the_system = cfg!(target_os = "macos") && std::env::args_os().len() <= 1;
    let Opening { mut root, mut others, file, place, more, wait } = resolve_args();
    // Restarted after an update: its windows again (the first one here, the rest once it's open).
    let mut reopen = if std::env::args_os().len() <= 1 { updates::windows_to_reopen() } else { Vec::new() };
    if !reopen.is_empty() {
        let mut first = reopen.remove(0).into_iter();
        root = first.next().unwrap_or(root);
        others = first.collect();
    }
    trace::mark("what to open worked out");
    // Off the Mac: a Null already running opens it instead (a waiting one is its own).
    let request =
        instance::Request { root: root.clone(), others: others.clone(), file: file.clone(), place, more: more.clone() };
    // A server's folder (`null ssh://box/app`): Null's own empty folder first, the server's
    // once it's reached (the window doesn't wait on it, and ssh may ask for a password).
    let (root, server) = match remote::is_remote(&root).then(session::scratch_folder).flatten() {
        Some(scratch) => (scratch, Some(root)),
        None => (root, None),
    };
    let mut handed = match instance::launch(&request, wait) {
        instance::Launch::HandedOver => return,
        instance::Launch::First(handed) => handed,
    };

    // Files and folders handed over by the Finder ("Open With", a drop on the Dock icon)
    // or by the `null` command while Null runs: opened in the window.
    let app = drawing::application().with_assets(assets::Assets);
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
                        open_project_window_now(root.clone(), None, None, Vec::new(), cx);
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
                        open_project_window_now(root.clone(), file, None, Vec::new(), cx);
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
        // What ssh asks (a password, whether to trust a server): asked in the window in
        // front, or the first one.
        let mut questions = askpass::questions();
        cx.spawn(async move |cx| {
            use futures::StreamExt;
            while let Some(ask) = questions.next().await {
                cx.update(|cx| {
                    let front = cx.active_window().and_then(|w| w.downcast::<Workspace>());
                    let window = front.or_else(|| cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>()));
                    // (No window: not answered, the connection gives up.)
                    if let Some(window) = window {
                        window.update(cx, |workspace, window, cx| workspace.ask_for_ssh(ask, window, cx)).ok();
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
                            let (file, place, more) = (file.clone(), request.place, request.more.clone());
                            handle
                                .update(cx, |workspace, window, cx| {
                                    window.activate_window();
                                    open_files(workspace, Some(file), place, more, window, cx);
                                })
                                .ok();
                            return;
                        }
                        // A server's folder: in the window in front, once the server's reached.
                        if remote::is_remote(&request.root)
                            && let Some(handle) = cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>())
                        {
                            let root = request.root.clone();
                            handle
                                .update(cx, |workspace, window, cx| {
                                    window.activate_window();
                                    workspace.open_paths(vec![root], window, cx);
                                })
                                .ok();
                            return;
                        }
                        let root = request.root.clone();
                        open_project_window_now(request.root, request.file, request.place, request.more, cx);
                        add_folders(&root, request.others, cx);
                    })
                    .ok();
                }
            })
            .detach();
        }
        // Local history past its month goes, now and then.
        std::thread::spawn(local_history::sweep);
        // An update put in place at the last quit leaves its files.
        std::thread::spawn(updates::tidy);
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
        cx.on_app_quit(|cx| {
            ollama::stop();
            instance::forget();
            updates::keep_windows(
                cx.windows()
                    .into_iter()
                    .filter_map(|w| w.downcast::<Workspace>())
                    .filter_map(|h| h.read(cx).ok().map(|w| w.roots().all().to_vec()))
                    .collect(),
            );
            updates::install_on_quit();
            session::release_own_lock();
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
                        open_project_window_now(root.clone(), file, place, more.clone(), cx);
                        reopen_server_project(&root, server.clone(), cx);
                    }
                })
                .ok();
            })
            .detach();
        } else if by_the_system {
            // (Already here: the paths open the window.)
        } else {
            let first = root.clone();
            open_project_window_now(root, file, place, more, cx);
            add_folders(&first, others, cx);
            reopen_server_project(&first, server, cx);
        }
        // (Each on its own: a server's waits for it.)
        for folders in reopen {
            let mut folders = folders.into_iter();
            let Some(first) = folders.next() else { continue };
            let (opened, server) = match remote::is_remote(&first).then(session::scratch_folder).flatten() {
                Some(scratch) => (scratch, Some(first)),
                None => (first, None),
            };
            open_project_window_now(opened.clone(), None, None, Vec::new(), cx);
            add_folders(&opened, folders.collect(), cx);
            reopen_server_project(&opened, server, cx);
        }
        trace::mark("window opened");
        // The spell checker started once the window's up, then what's shown checked.
        cx.spawn(async move |cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
            cx.background_executor().spawn(async { spell::load() }).await;
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
    cx.defer(move |cx| open_project_window_now(root, file, None, Vec::new(), cx));
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

/// The files asked for opened in `workspace`, in order, the first in front.
fn open_files(
    workspace: &mut Workspace,
    file: Option<PathBuf>,
    place: Option<(u32, u32)>,
    more: Vec<(PathBuf, Option<(u32, u32)>)>,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) {
    let Some(file) = file else { return };
    let again = (!more.is_empty()).then(|| file.clone());
    open_at(workspace, file, place, window, cx);
    for (other, place) in more {
        open_at(workspace, other, place, window, cx);
    }
    // (Back to the first: open, it's only shown.)
    if let Some(first) = again {
        workspace.open_file(first, window, cx);
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
    if let Err(error) = opened {
        if cx.windows().is_empty() {
            drawing::window_failed(&format!("{error:#}"));
        }
        cx.quit();
    }
}

/// Started on Null's own empty folder for a server's (`server`, or the one it last closed
/// on): that folder, once the server's reached (the window doesn't wait on it).
fn reopen_server_project(root: &std::path::Path, server: Option<PathBuf>, cx: &mut gpui::App) {
    if !session::is_scratch(root) {
        return;
    }
    let (Some(server), Some(handle)) = (server.or_else(session::last_server_project), window_on(root, None, cx)) else {
        return;
    };
    handle.update(cx, |workspace, window, cx| workspace.reopen_server_project(server, window, cx)).ok();
}

fn open_project_window_now(
    root: PathBuf,
    file: Option<PathBuf>,
    place: Option<(u32, u32)>,
    more: Vec<(PathBuf, Option<(u32, u32)>)>,
    cx: &mut gpui::App,
) {
    if let Some(handle) = window_on(&root, None, cx) {
        handle
            .update(cx, |workspace, window, cx| {
                window.activate_window();
                open_files(workspace, file, place, more, window, cx);
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
            let workspace = Workspace::new(root, window, cx);
            window.focus(&workspace.focus_handle(cx));
            workspace
        })
    });
    match opened {
        // The project's files once the window is up: on Linux (GNOME over remote desktop,
        // drawn without a graphics card) a window whose first frame had a file in it was
        // never drawn again until a key or the mouse came.
        Ok(handle) => {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
                handle
                    .update(cx, |workspace, window, cx| {
                        workspace.restore_session(session, window, cx);
                        open_files(workspace, file, place, more, window, cx);
                        // (Settings that can't be read are yours, not none: no welcome over
                        // them each time, its choices couldn't be saved; the notice says why.)
                        if !cx.global::<settings::Settings>().welcomed && settings::Settings::unreadable().is_none() {
                            workspace.show_welcome(window, cx);
                        }
                    })
                    .ok();
            })
            .detach();
        }
        Err(error) => {
            eprintln!("null: couldn't open a window: {error}");
            // Nothing open, nothing would ever close: said where it's seen (or drawn without
            // the card, on Linux), not left running out of sight.
            if cx.windows().is_empty() {
                drawing::window_failed(&format!("{error:#}"));
            }
        }
    }
}

/// Quitting: the next window with unsaved changes (or a command running in a terminal)
/// asks about them; once none is left, Null quits.
/// Quitting was called off in one window: none of them counts as done asking any more.
pub(crate) fn quit_cancelled(cx: &mut gpui::App) {
    updates::restart_called_off();
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
    fn a_server_s_folder_is_given_as_an_ssh_address() {
        assert_eq!(server_folder("ssh://me@box:2222/srv/app"), Some(remote::path("me@box#2222", "/srv/app")));
        assert_eq!(server_folder("ssh://box"), Some(remote::path("box", "/")));
        assert_eq!(server_folder("/ssh:box:/x"), Some(PathBuf::from("/ssh:box:/x")));
        assert_eq!(server_folder("ssh://"), None);
        assert_eq!(server_folder("src/main.rs"), None);
    }

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
    fn options_anywhere_and_unknown_ones_refused() {
        let args = |list: &[&str]| parse_args(&list.iter().map(std::ffi::OsString::from).collect::<Vec<_>>());
        let asked = args(&["a.rs", "--wait"]).unwrap();
        assert!(asked.wait);
        assert_eq!(asked.paths, ["a.rs"]);
        assert!(args(&["a.rs", "-h"]).unwrap().help);
        assert!(args(&["--frobnicate"]).unwrap_err().contains("--frobnicate"));
        // After `--`, a file named like an option.
        assert_eq!(args(&["--", "-notes.md", "--wait"]).unwrap().paths, ["-notes.md", "--wait"]);
        assert_eq!(args(&["-"]).unwrap().paths, ["-"]);
    }

    /// A file name that isn't UTF-8 (Latin-1's `café`) is a path, kept byte for byte.
    #[cfg(unix)]
    #[test]
    fn a_name_that_is_not_utf8_is_kept_as_it_is() {
        use std::os::unix::ffi::OsStringExt;
        let name = std::ffi::OsString::from_vec(b"caf\xe9.txt".to_vec());
        let asked = parse_args(&[name.clone(), "--wait".into()]).unwrap();
        assert!(asked.wait);
        assert_eq!(asked.paths, [name]);
    }

    /// A file given from a terminal in a folder: that folder as the project, unless it's too
    /// wide (the home folder, `/`) or above the file's repository.
    #[test]
    fn a_file_s_project_from_where_it_was_asked() {
        let dir = absolute(&crate::tools::test_dir("file-root"));
        let repo = dir.join("code/app");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("plain/src")).unwrap();
        let file = repo.join("src/main.rs");
        assert_eq!(root_for_file(&repo, &file), repo);
        assert_eq!(root_for_file(&repo.join("src"), &file), repo.join("src"));
        assert_eq!(root_for_file(&dir.join("code"), &file), repo, "above the repository");
        assert_eq!(root_for_file(Path::new("/"), &file), repo, "the whole disk");
        // No repository: the folder asked from, or the file's own when it's elsewhere.
        let plain = dir.join("plain/src/a.rs");
        assert_eq!(root_for_file(&dir.join("plain"), &plain), dir.join("plain"));
        assert_eq!(root_for_file(&repo, &plain), dir.join("plain/src"));
        std::fs::remove_dir_all(&dir).ok();
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
