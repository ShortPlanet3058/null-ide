//! The built-in terminal: a real shell in a PTY, emulated by `alacritty_terminal`
//! and drawn with GPUI.

use crate::fonts::Fonts;
use crate::theme::Theme;
use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, Bounds, ClipboardItem, Context, Element, ElementId, Entity, EventEmitter, FocusHandle, Focusable, Font,
    FontStyle, FontWeight, GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyBinding, KeyDownEvent, Keystroke,
    LayoutId, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, NoAction, Pixels, Point,
    ScrollDelta, ScrollWheelEvent, ShapedLine, Style, Task, TextRun, UnderlineStyle, Window, actions, div, fill, font,
    point, prelude::*, px, relative, size,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

actions!(terminal, [Copy, Paste, Clear, Find, FindNext, FindPrevious, CloseFind]);

/// The terminal's text is a point smaller than the editor's, and follows it (⌘+ / ⌘-).
const FONT_SIZE_BELOW_EDITOR: f32 = 1.;
const PADDING: f32 = 10.;
const SCROLLBACK: usize = 10_000;

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Terminal");
    let mut keys = Vec::new();
    if cfg!(target_os = "macos") {
        keys.extend([
            KeyBinding::new("cmd-c", Copy, ctx),
            KeyBinding::new("cmd-v", Paste, ctx),
            KeyBinding::new("cmd-k", Clear, ctx),
            KeyBinding::new("cmd-f", Find, ctx),
            KeyBinding::new("cmd-g", FindNext, ctx),
            KeyBinding::new("cmd-shift-g", FindPrevious, ctx),
        ]);
    } else {
        keys.extend([
            KeyBinding::new("ctrl-shift-c", Copy, ctx),
            KeyBinding::new("ctrl-shift-v", Paste, ctx),
            KeyBinding::new("ctrl-shift-f", Find, ctx),
        ]);
        // Outside macOS, Null's shortcuts use Ctrl, which shells need too (Ctrl+W
        // deletes a word, Ctrl+B moves back...). Inside the terminal, give them back.
        for key in "abcdefghijklmnopqrstuvwxyz,=-0".chars() {
            keys.push(KeyBinding::new(&format!("ctrl-{key}"), NoAction {}, ctx));
            // Ctrl+Shift+C and V stay copy and paste.
            if key != 'c' && key != 'v' {
                keys.push(KeyBinding::new(&format!("ctrl-shift-{key}"), NoAction {}, ctx));
            }
        }
    }
    // In the find field: Enter and Shift+Enter step through, Escape goes back to the shell.
    let find = Some("TerminalFind");
    keys.extend([
        KeyBinding::new("enter", FindNext, find),
        KeyBinding::new("shift-enter", FindPrevious, find),
        KeyBinding::new("escape", CloseFind, find),
    ]);
    cx.bind_keys(keys);
}

/// Searching the terminal's output (⌘F): the field, and what it found.
struct TerminalFind {
    input: Entity<crate::text_input::TextInput>,
    /// Each match: its row (as the grid counts, history negative) and columns.
    matches: Vec<(i32, std::ops::Range<usize>)>,
    current: Option<usize>,
    /// After new output, the search runs again once it pauses.
    refresh: Option<Task<()>>,
    _subscription: gpui::Subscription,
}

/// Where `query` appears in `rows` (case only counts when the query has capitals).
fn find_in_rows(rows: &[(i32, Vec<char>)], query: &str) -> Vec<(i32, std::ops::Range<usize>)> {
    const MAX: usize = 10_000;
    if query.is_empty() {
        return Vec::new();
    }
    let exact = query.chars().any(char::is_uppercase);
    let fold = |c: char| if exact { c } else { c.to_lowercase().next().unwrap_or(c) };
    let needle: Vec<char> = query.chars().map(fold).collect();
    let mut found = Vec::new();
    for (line, row) in rows {
        let row: Vec<char> = row.iter().map(|&c| fold(c)).collect();
        let mut at = 0;
        while at + needle.len() <= row.len() {
            if row[at..at + needle.len()] == needle[..] {
                found.push((*line, at..at + needle.len()));
                if found.len() >= MAX {
                    return found;
                }
                at += needle.len();
            } else {
                at += 1;
            }
        }
    }
    found
}

/// Forwards the emulator's events to the UI thread.
#[derive(Clone)]
struct Listener(mpsc::UnboundedSender<TermEvent>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        self.0.unbounded_send(event).ok();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct GridSize {
    columns: usize,
    lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// A new shell has started once its output is quiet this long…
const SETTLE_QUIET: std::time::Duration = std::time::Duration::from_millis(300);
/// …or, if it says nothing at all, after this long.
const SETTLE_AT_MOST: std::time::Duration = std::time::Duration::from_secs(4);

pub enum TerminalEvent {
    TitleChanged,
    Exited,
    /// ⌘-click on a place in a file the output names: the file, and its 1-based line and column.
    OpenFile(PathBuf, Option<u32>, Option<u32>),
    /// A command that ran a while finished: its name, and how long it took.
    Finished(String, std::time::Duration),
    /// A command finished: what it was, and what it printed.
    Ran(Ran),
}

/// A command that finished in the shell.
#[derive(Clone, Debug)]
pub struct Ran {
    /// What to call it: the program (`cargo`).
    pub name: String,
    /// The line typed for it (`cargo build`), without the prompt: the same command run again
    /// has the same.
    pub command: String,
    /// The shell's folder when it began: relative paths in its output are from there.
    pub folder: PathBuf,
    pub output: String,
}

/// Where a command began: its line (see `output_mark`), and that line's text to find it
/// by once the history is full and lines no longer count up; the shell's folder then;
/// when; and whether the program was seen running.
struct RunStart {
    mark: usize,
    line: String,
    folder: PathBuf,
    at: std::time::Instant,
    seen: bool,
}

/// A command too quick to be seen running is taken as done this long after Return.
const QUICK_RUN: std::time::Duration = std::time::Duration::from_millis(600);

/// A row's text as it reads: a wide character once (not followed by its spacer), with the
/// marks drawn over it.
fn row_text(grid: &alacritty_terminal::grid::Grid<alacritty_terminal::term::cell::Cell>, line: Line) -> String {
    row_text_to(grid, line, grid.columns())
}

/// A row's text up to column `end` (see `row_text`).
fn row_text_to(
    grid: &alacritty_terminal::grid::Grid<alacritty_terminal::term::cell::Cell>,
    line: Line,
    end: usize,
) -> String {
    let row = &grid[line];
    let mut text = String::new();
    for column in 0..end.min(grid.columns()) {
        let cell = &row[Column(column)];
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        text.push(cell.c);
        text.extend(cell.zerowidth().into_iter().flatten());
    }
    text
}

/// The signs a prompt ends with, before what's typed.
const PROMPT_SIGNS: [&str; 6] = ["% ", "$ ", "# ", "> ", "❯ ", "➜ "];

/// The command typed on a prompt's line (`me@mac app % make test` → `make test`): what
/// follows the prompt's first `% `, `$ `, `# `, `> `, `❯ ` or `➜ `, from its first word
/// that's a program (`➜  app git:(main) make` → `make`), or the whole line; without what a
/// prompt shows at the right, after a long gap (`make   at 10:42`).
fn typed_command(line: &str) -> &str {
    let end = PROMPT_SIGNS.iter().filter_map(|sign| line.find(sign).map(|at| at + sign.len())).min().unwrap_or(0);
    let typed = line[end..].trim();
    let typed = typed.split("   ").next().unwrap_or(typed).trim_end();
    let paths: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let program = |word: &str| {
        word.contains('=')
            || word.starts_with("./")
            || word.starts_with("../")
            || (word.starts_with('/') && std::path::Path::new(word).is_file())
            || (!word.contains('/') && paths.iter().any(|dir| dir.join(word).is_file()))
    };
    let mut at = 0;
    for word in typed.split_whitespace() {
        let start = typed[at..].find(word).map_or(at, |i| at + i);
        if program(word) {
            return &typed[start..];
        }
        at = start + word.len();
    }
    typed
}

/// The program a typed command runs (`FOO=1 ./build.sh -v` → `build.sh`).
fn program_of(command: &str) -> &str {
    let word = command.split_whitespace().find(|w| !w.contains('=')).unwrap_or("");
    word.rsplit('/').next().unwrap_or(word)
}

/// A link the mouse is over while ⌘ is held: its row (as the grid counts, history
/// negative) and columns, and what it opens.
struct HoveredLink {
    line: i32,
    columns: std::ops::Range<usize>,
    target: LinkTarget,
}

#[derive(Clone)]
enum LinkTarget {
    Url(String),
    File(PathBuf, Option<u32>, Option<u32>),
}

pub struct TerminalView {
    focus_handle: FocusHandle,
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    size: GridSize,
    cell: (Pixels, Pixels),
    pub title: String,
    /// A name given to it, over the one its folder gives.
    pub name: Option<String>,
    /// Where the command running (or just typed) began.
    run: Option<RunStart>,
    /// The program running in the shell now (`cargo`), while one is.
    pub running: Option<String>,
    selecting: bool,
    /// While a program gets the mouse: the button held down (if any), and the last cell
    /// reported, so moving within a cell sends nothing.
    mouse_held: Option<MouseButton>,
    mouse_cell: Option<(usize, usize)>,
    /// Scrolling smaller than a line, kept for the next event.
    scroll_rest: f32,
    /// Where the grid was drawn last frame, for mouse selection.
    origin: Point<Pixels>,
    /// Commands waiting for a new shell to finish starting, so they aren't echoed early.
    queued: Vec<String>,
    /// Typed once the shell is ready, before anything queued, and not followed as a run of
    /// its own (activating a Python environment).
    setup: Option<String>,
    /// Whether the shell has started (its output went quiet after the first prompt).
    settled: bool,
    /// Whether the shell has written anything yet.
    spoke: bool,
    settle_task: Option<Task<()>>,
    /// Output shown at most about 30 times a second (each is the whole window drawn): when
    /// it was last, and the one waiting.
    last_redraw: std::time::Instant,
    redraw_task: Option<Task<()>>,
    /// Where the shell started: relative paths in its output are found from here.
    root: PathBuf,
    link: Option<HoveredLink>,
    find: Option<TerminalFind>,
    _events: Task<()>,
    _watching: Option<Task<()>>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

/// A shell that's running, before its view exists.
pub struct Shell {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    events: mpsc::UnboundedReceiver<TermEvent>,
    size: GridSize,
    cwd: PathBuf,
    watch: Option<crate::terminal_watch::Foreground>,
}

impl Shell {
    /// Starts the user's shell in `cwd`.
    pub fn start(cwd: PathBuf) -> std::io::Result<Self> {
        // Without `$SHELL` (an app started oddly), the account's own shell rather than a
        // `login` wrapper, so the shell is the process watched for its prompt.
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).or_else(crate::terminal_watch::account_shell);
        let mut env = HashMap::from([
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
            ("TERM_PROGRAM".to_string(), "Null".to_string()),
        ]);
        // Apps started from the Dock often have no locale, and shells then mangle
        // accented characters. Use UTF-8 unless one is already set, as Terminal.app does.
        if ["LANG", "LC_ALL", "LC_CTYPE"].iter().all(|v| std::env::var_os(v).is_none_or(|x| x.is_empty())) {
            env.insert("LANG".into(), "en_US.UTF-8".into());
        }
        let options = tty::Options {
            // A login shell, like a terminal app opens, so PATH and the prompt are set up.
            shell: shell.map(|program| tty::Shell::new(program, vec!["-l".into()])),
            working_directory: Some(cwd.clone()),
            drain_on_exit: false,
            env,
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let size = GridSize { columns: 80, lines: 24 };
        let window_size = WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 };
        let pty = tty::new(&options, window_size, 0)?;
        #[cfg(unix)]
        let watch = crate::terminal_watch::Foreground::new(pty.file(), pty.child().id());
        #[cfg(not(unix))]
        let watch = None;
        let (tx, events) = mpsc::unbounded();
        let listener = Listener(tx);
        let config = Config { scrolling_history: SCROLLBACK, ..Default::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Self { term, sender, events, size, cwd, watch })
    }
}

impl TerminalView {
    pub fn new(shell: Shell, cx: &mut Context<Self>) -> Self {
        let Shell { term, sender, mut events, size, cwd, watch } = shell;
        // Once a second, what the shell is running.
        let watching = watch.map(|watch| {
            cx.spawn(async move |this, cx| {
                let mut running = None;
                loop {
                    cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
                    let now = watch.program();
                    let name = now.as_ref().map(|(_, name)| name.clone());
                    let finished = crate::terminal_watch::step(
                        &mut running,
                        now,
                        std::time::Instant::now(),
                        crate::terminal_watch::group_alive,
                    );
                    let Ok(()) = this.update(cx, |this, cx| {
                        if this.running != name {
                            // Started: where its output begins (from Return, if it was seen).
                            // Ended: what it printed.
                            match (this.running.take(), &name) {
                                (None, Some(_)) => {
                                    if this.run.is_none() {
                                        this.run = Some(this.run_start(None));
                                    }
                                    if let Some(run) = &mut this.run {
                                        run.seen = true;
                                    }
                                }
                                (Some(ran), None) => {
                                    if let Some(run) = this.run.take() {
                                        this.report_run(Some(ran), run, cx);
                                    }
                                }
                                _ => {}
                            }
                            this.running = name;
                            cx.emit(TerminalEvent::TitleChanged);
                        } else if this.running.is_none()
                            && this
                                .run
                                .as_ref()
                                .is_some_and(|r| !r.seen && r.at.elapsed() >= QUICK_RUN && this.at_prompt(r))
                            && let Some(run) = this.run.take()
                        {
                            // Done before it could be seen (a quick `gcc`), and the prompt is
                            // back (not still in the shell's own work, as `nvm use`): it ran.
                            this.report_run(None, run, cx);
                        }
                        if let Some((name, took)) = finished {
                            cx.emit(TerminalEvent::Finished(name, took));
                        }
                    }) else {
                        break;
                    };
                }
            })
        });
        let events = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this.update(cx, |this, cx| this.handle_event(event, cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            focus_handle: cx.focus_handle(),
            term,
            sender,
            size,
            cell: (px(8.), px(16.)),
            title: String::new(),
            name: None,
            run: None,
            running: None,
            selecting: false,
            mouse_held: None,
            mouse_cell: None,
            scroll_rest: 0.,
            origin: Point::default(),
            queued: Vec::new(),
            setup: None,
            settled: false,
            spoke: false,
            settle_task: None,
            last_redraw: std::time::Instant::now(),
            redraw_task: None,
            root: cwd,
            link: None,
            find: None,
            _events: events,
            _watching: watching,
        }
    }

    /// Types `command` into the shell and presses Return; in a shell still starting,
    /// once it has.
    pub fn run_command(&mut self, command: &str, cx: &mut Context<Self>) {
        if self.settled {
            if self.running.is_none() {
                self.run = Some(self.run_start(Some(command)));
            }
            return self.write(format!("{command}\r").into_bytes());
        }
        self.queued.push(command.to_string());
        // Before its first prompt, wait for it (but not forever: a silent shell gets it anyway).
        let wait = if self.spoke { SETTLE_QUIET } else { SETTLE_AT_MOST };
        self.settle_after(wait, cx);
    }

    /// Typed into the shell as it starts, before any command: not a run of its own (no
    /// notice, nothing read from what it prints), and what's run after waits for it.
    pub fn prepare(&mut self, command: &str, cx: &mut Context<Self>) {
        if self.settled {
            return self.write(format!("{command}\r").into_bytes());
        }
        self.setup = Some(command.to_string());
        let wait = if self.spoke { SETTLE_QUIET } else { SETTLE_AT_MOST };
        self.settle_after(wait, cx);
    }

    /// The shell counts as started once its output has been quiet a moment.
    fn settle_after(&mut self, wait: std::time::Duration, cx: &mut Context<Self>) {
        self.settle_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                // Its setup first; what's queued once the shell is quiet again after it.
                if let Some(setup) = this.setup.take() {
                    this.write(format!("{setup}\r").into_bytes());
                    return this.settle_after(SETTLE_AT_MOST, cx);
                }
                this.settled = true;
                for command in std::mem::take(&mut this.queued) {
                    this.run = Some(this.run_start(Some(&command)));
                    this.write(format!("{command}\r").into_bytes());
                }
            })
            .ok();
        }));
    }

    /// New output shown: now, or (a stream of it) a moment after the last time.
    fn redraw_soon(&mut self, cx: &mut Context<Self>) {
        const EVERY: std::time::Duration = std::time::Duration::from_millis(33);
        if self.redraw_task.is_some() {
            return;
        }
        let since = self.last_redraw.elapsed();
        if since >= EVERY {
            self.last_redraw = std::time::Instant::now();
            return cx.notify();
        }
        self.redraw_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(EVERY - since).await;
            this.update(cx, |this, cx| {
                this.redraw_task = None;
                this.last_redraw = std::time::Instant::now();
                cx.notify();
            })
            .ok();
        }));
    }

    fn handle_event(&mut self, event: TermEvent, cx: &mut Context<Self>) {
        match event {
            TermEvent::Wakeup => {
                if let Some(find) = &mut self.find
                    && find.refresh.is_none()
                {
                    // While output streams, its matches are found again a little after (each
                    // look reads the whole history, under the terminal's lock).
                    find.refresh = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(std::time::Duration::from_millis(600)).await;
                        this.update(cx, |this, cx| {
                            if let Some(find) = &mut this.find {
                                find.refresh = None;
                            }
                            this.search(false, cx);
                        })
                        .ok();
                    }));
                }
                // Output while starting: the prompt is coming; wait for it to go quiet.
                if !self.settled {
                    self.spoke = true;
                    self.settle_after(SETTLE_QUIET, cx);
                }
                self.redraw_soon(cx)
            }
            TermEvent::CursorBlinkingChange | TermEvent::MouseCursorDirty => cx.notify(),
            TermEvent::Title(title) => {
                self.title = title;
                cx.emit(TerminalEvent::TitleChanged);
                cx.notify();
            }
            TermEvent::ResetTitle => {
                self.title.clear();
                cx.emit(TerminalEvent::TitleChanged);
            }
            TermEvent::PtyWrite(text) => self.write(text.into_bytes()),
            TermEvent::ClipboardStore(_, text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            TermEvent::ClipboardLoad(_, format) => {
                let text = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
                self.write(format(&text).into_bytes());
            }
            TermEvent::TextAreaSizeRequest(format) => {
                let reply = format(self.window_size());
                self.write(reply.into_bytes());
            }
            TermEvent::ColorRequest(index, format) => {
                let theme = cx.global::<Theme>();
                let color = indexed_color(index, theme).to_rgb();
                let rgb = Rgb { r: (color.r * 255.) as u8, g: (color.g * 255.) as u8, b: (color.b * 255.) as u8 };
                self.write(format(rgb).into_bytes());
            }
            TermEvent::ChildExit(_) | TermEvent::Exit => cx.emit(TerminalEvent::Exited),
            TermEvent::Bell => {}
        }
    }

    /// Runs `text` as if typed and entered. A shell that takes pasted text whole gets it so
    /// (tabs and all, as ⌘V gives it), then Return; otherwise, or one still starting, line
    /// after line, with no control characters: a tab would ask the shell for completions,
    /// ⌃D could end it.
    pub fn run_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.term.lock().scroll_display(Scroll::Bottom);
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        if self.settled && bracketed {
            let text = text.replace('\x1b', "").replace("\r\n", "\n").replace('\n', "\r");
            let text = text.trim_end_matches('\r');
            if self.running.is_none() {
                self.run = Some(self.run_start(text.split('\r').next()));
            }
            return self.write(format!("\x1b[200~{text}\x1b[201~\r").into_bytes());
        }
        for line in run_lines(text) {
            self.run_command(&line, cx);
        }
    }

    pub(crate) fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.sender.send(Msg::Input(bytes.into())).ok();
    }

    fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: self.size.lines as u16,
            num_cols: self.size.columns as u16,
            cell_width: f32::from(self.cell.0) as u16,
            cell_height: f32::from(self.cell.1) as u16,
        }
    }

    /// Fits the grid to the space it's drawn in.
    fn resize(&mut self, size: GridSize, cell: (Pixels, Pixels)) {
        if size != self.size || cell != self.cell {
            self.size = size;
            self.cell = cell;
            self.term.lock().resize(size);
            self.sender.send(Msg::Resize(self.window_size())).ok();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Typing in the find field isn't for the shell.
        if !self.focus_handle.is_focused(window) {
            return;
        }
        let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = key_to_bytes(&event.keystroke, app_cursor) {
            // Return at the shell's prompt: a command begins here.
            if bytes == b"\r" && self.running.is_none() {
                self.run = Some(self.run_start(None));
            }
            // Typing jumps back to the prompt if the view was scrolled up.
            self.term.lock().scroll_display(Scroll::Bottom);
            self.term.lock().selection = None;
            self.write(bytes);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.term.lock().selection_to_string().filter(|t| !t.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        // No escape characters: pasted text could otherwise end bracketed paste early and run commands.
        let text = text.replace('\x1b', "").replace("\r\n", "\r").replace('\n', "\r");
        let bytes = if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text };
        self.term.lock().scroll_display(Scroll::Bottom);
        self.write(bytes.into_bytes());
    }

    fn clear(&mut self, _: &Clear, _: &mut Window, cx: &mut Context<Self>) {
        // Ctrl+L redraws the prompt at the top; the scrollback stays.
        self.write(b"\x0c".to_vec());
        cx.notify();
    }

    /// ⌘F: the find field over the output, with what's selected in it (or the last search).
    fn open_find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            let input = cx.new(|cx| crate::text_input::TextInput::new("Find in output", cx));
            let subscription =
                cx.subscribe(&input, |this, _, _: &crate::text_input::TextInputEvent, cx| this.search(true, cx));
            self.find = Some(TerminalFind {
                input,
                matches: Vec::new(),
                current: None,
                refresh: None,
                _subscription: subscription,
            });
        }
        let selected = self.term.lock().selection_to_string().filter(|s| !s.is_empty() && !s.contains('\n'));
        if let Some(find) = &self.find {
            find.input.update(cx, |input, cx| {
                if let Some(text) = &selected {
                    input.set_text(text, cx);
                }
                input.select_all_text(cx);
            });
            window.focus(&find.input.focus_handle(cx));
        }
        self.search(true, cx);
    }

    fn close_find(&mut self, _: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
        self.find = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// The shell's folder now, as its title shows it ("me@mac:~/code/app"), or the one
    /// it started in.
    fn shell_folder(&self) -> PathBuf {
        let shown = self.title.rsplit(':').next().map(str::trim).unwrap_or("");
        let shown = match (shown.strip_prefix("~/"), std::env::var_os("HOME")) {
            (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
            _ => PathBuf::from(shown),
        };
        if shown.is_absolute() && shown.is_dir() { shown } else { self.root.clone() }
    }

    /// A command beginning now, at the cursor's line.
    /// A command beginning at the cursor's line (its first row, when a long one wraps).
    /// `typing`: the command Null is about to type there itself, not on the line yet.
    fn run_start(&self, typing: Option<&str>) -> RunStart {
        let (mark, line) = {
            let term = self.term.lock();
            let grid = term.grid();
            let cursor = grid.cursor.point;
            let top = -(grid.history_size() as i32);
            let wraps = |line: i32| grid[Line(line)][Column(grid.columns() - 1)].flags.contains(Flags::WRAPLINE);
            let mut first = cursor.line.0;
            while first > top && wraps(first - 1) {
                first -= 1;
            }
            let text = match typing {
                Some(command) if first == cursor.line.0 => {
                    format!("{}{command}", row_text_to(grid, cursor.line, cursor.column.0))
                }
                _ => row_text(grid, Line(first)),
            };
            // As the line's first row will read: to find it again by.
            let text: String = text.chars().take(grid.columns()).collect();
            ((grid.history_size() as i32 + first).max(0) as usize, text.trim_end().to_string())
        };
        RunStart { mark, line, folder: self.shell_folder(), at: std::time::Instant::now(), seen: false }
    }

    /// Whether the shell shows its prompt again after `run`: what's before the cursor
    /// begins as the prompt `run` was typed on did (its first word: `➜`, `ada@mac`), or
    /// ends as a prompt does.
    fn at_prompt(&self, run: &RunStart) -> bool {
        let term = self.term.lock();
        let grid = term.grid();
        let cursor = grid.cursor.point;
        let before = row_text_to(grid, cursor.line, cursor.column.0);
        // What came before the command on its line (`typed_command` gives part of it).
        let typed = typed_command(&run.line);
        let at = (typed.as_ptr() as usize).saturating_sub(run.line.as_ptr() as usize).min(run.line.len());
        let prompt = run.line.get(..at).unwrap_or(&run.line);
        let same_start = prompt.split_whitespace().next().is_some_and(|w| before.split_whitespace().next() == Some(w));
        same_start || PROMPT_SIGNS.iter().any(|sign| before.ends_with(sign))
    }

    /// Tells what a finished command printed (unless nothing was typed: a bare Return).
    fn report_run(&self, program: Option<String>, run: RunStart, cx: &mut Context<Self>) {
        let output = self.output_since(&run);
        let command = typed_command(output.lines().next().unwrap_or("")).to_string();
        let name = program.unwrap_or_else(|| program_of(&command).to_string());
        if name.is_empty() {
            return;
        }
        cx.emit(TerminalEvent::Ran(Ran { name, command, folder: run.folder, output }));
    }

    /// The text from where `run` began to the cursor: a row wrapped onto the next joined to
    /// it, the last 5,000 lines at most.
    fn output_since(&self, run: &RunStart) -> String {
        let term = self.term.lock();
        let grid = term.grid();
        let history = grid.history_size() as i32;
        let last = grid.cursor.point.line.0;
        let lowest = (-history).max(last - 5000);
        let mut first = run.mark as i32 - history;
        // A full history no longer counts lines up: the line it began on, by its text.
        if grid.history_size() >= SCROLLBACK && !run.line.is_empty() {
            first = (lowest..=last).rev().find(|&l| row_text(grid, Line(l)).starts_with(&run.line)).unwrap_or(first);
        }
        let mut out = String::new();
        for line in first.max(lowest)..=last {
            let row = &grid[Line(line)];
            let text = row_text(grid, Line(line));
            let wrapped = row[Column(grid.columns() - 1)].flags.contains(Flags::WRAPLINE);
            out.push_str(if wrapped { &text } else { text.trim_end() });
            if !wrapped {
                out.push('\n');
            }
        }
        out
    }

    /// Looks through the output (history included) for the field's text. A new search
    /// goes to the match nearest the bottom; one after new output keeps its place.
    fn search(&mut self, new: bool, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else { return };
        let query = find.input.read(cx).text().to_string();
        // Nothing to find: nothing to read.
        if query.is_empty() {
            find.matches.clear();
            find.current = None;
            return cx.notify();
        }
        let rows: Vec<(i32, Vec<char>)> = {
            let term = self.term.lock();
            let grid = term.grid();
            let (top, bottom) = (-(grid.history_size() as i32), grid.screen_lines() as i32);
            (top..bottom)
                .map(|line| {
                    let row = &grid[Line(line)];
                    (line, (0..grid.columns()).map(|c| row[Column(c)].c).collect())
                })
                .collect()
        };
        let matches = find_in_rows(&rows, &query);
        let Some(find) = &mut self.find else { return };
        let was = find.current.and_then(|i| find.matches.get(i)).map(|(line, _)| *line);
        find.current = match was.filter(|_| !new) {
            Some(line) => matches.iter().position(|(l, _)| *l >= line).or(matches.len().checked_sub(1)),
            None => matches.len().checked_sub(1),
        };
        find.matches = matches;
        // After new output, the view stays where it is; a new search goes to its match.
        if new {
            self.reveal_match();
        }
        cx.notify();
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else { return };
        let count = find.matches.len();
        if count == 0 {
            return;
        }
        find.current = Some(match (find.current, forward) {
            (Some(i), true) => (i + 1) % count,
            (Some(i), false) => (i + count - 1) % count,
            (None, _) => count - 1,
        });
        self.reveal_match();
        cx.notify();
    }

    /// Scrolls so the current match is on screen, in the middle when it wasn't.
    fn reveal_match(&mut self) {
        let Some((line, _)) = self.find.as_ref().and_then(|f| f.current.and_then(|i| f.matches.get(i)).cloned()) else {
            return;
        };
        let mut term = self.term.lock();
        let offset = term.grid().display_offset() as i32;
        let lines = term.grid().screen_lines() as i32;
        let shown = line + offset;
        if !(0..lines).contains(&shown) {
            let wanted = (lines / 2 - line).clamp(0, term.grid().history_size() as i32);
            term.scroll_display(Scroll::Delta(wanted - offset));
        }
    }

    fn grid_point(&self, position: Point<Pixels>) -> (GridPoint, Side) {
        let x = f32::from(position.x - self.origin.x).max(0.);
        let y = f32::from(position.y - self.origin.y).max(0.);
        let (cw, ch) = (f32::from(self.cell.0), f32::from(self.cell.1));
        let column = ((x / cw) as usize).min(self.size.columns.saturating_sub(1));
        let side = if (x / cw).fract() < 0.5 { Side::Left } else { Side::Right };
        let offset = self.term.lock().grid().display_offset() as i32;
        let line = ((y / ch) as i32).min(self.size.lines as i32 - 1) - offset;
        (GridPoint::new(Line(line), Column(column)), side)
    }

    /// The cell under the mouse on screen: (column, line), from the top-left.
    fn screen_cell(&self, position: Point<Pixels>) -> (usize, usize) {
        let x = f32::from(position.x - self.origin.x).max(0.);
        let y = f32::from(position.y - self.origin.y).max(0.);
        let column = ((x / f32::from(self.cell.0)) as usize).min(self.size.columns.saturating_sub(1));
        let line = ((y / f32::from(self.cell.1)) as usize).min(self.size.lines.saturating_sub(1));
        (column, line)
    }

    /// Whether the program running asked for the mouse (vim, htop, tmux...). Holding Shift
    /// keeps it for selecting text, as in other terminals.
    fn program_wants_mouse(&self, modifiers: &Modifiers) -> bool {
        // ⌘ is for links, even in a program that took the mouse.
        self.term.lock().mode().intersects(TermMode::MOUSE_MODE) && !modifiers.shift && !modifiers.secondary()
    }

    /// The link under `position`, if the output there names a web address or a file
    /// that exists (from where the shell started, or the folder its title shows).
    fn link_at(&self, position: Point<Pixels>) -> Option<HoveredLink> {
        let (point, _) = self.grid_point(position);
        let row: Vec<char> = {
            let term = self.term.lock();
            let grid = term.grid();
            if point.line.0 < -(grid.history_size() as i32) || point.line.0 >= grid.screen_lines() as i32 {
                return None;
            }
            let row = &grid[point.line];
            (0..grid.columns())
                .map(|c| {
                    let cell = &row[Column(c)];
                    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) { ' ' } else { cell.c }
                })
                .collect()
        };
        let (columns, link) = crate::terminal_links::link_at(&row, point.column.0)?;
        let target = match link {
            crate::terminal_links::Link::Url(url) => LinkTarget::Url(url),
            crate::terminal_links::Link::File { path, line, column } => {
                LinkTarget::File(self.find_file(&path)?, line, column)
            }
        };
        Some(HoveredLink { line: point.line.0, columns, target })
    }

    fn find_file(&self, path: &str) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let expand = |p: &str| match (p.strip_prefix("~/"), &home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(p),
        };
        let path = expand(path);
        if path.is_absolute() {
            return path.is_file().then_some(path);
        }
        // The shell's folder, as its title shows it ("me@mac:~/code/app"), then where it started.
        let shown = self.title.rsplit(':').next().map(|t| expand(t.trim())).filter(|d| d.is_absolute());
        shown.into_iter().chain([self.root.clone()]).map(|dir| dir.join(&path)).find(|p| p.is_file())
    }

    fn update_link(&mut self, position: Point<Pixels>, modifiers: &Modifiers, cx: &mut Context<Self>) {
        let link = if modifiers.secondary() { self.link_at(position) } else { None };
        let same = match (&self.link, &link) {
            (Some(a), Some(b)) => a.line == b.line && a.columns == b.columns,
            (None, None) => true,
            _ => false,
        };
        self.link = link;
        if !same {
            cx.notify();
        }
    }

    /// Tells the program about a mouse event, in the encoding it asked for.
    fn report_mouse(&mut self, button: u8, position: Point<Pixels>, modifiers: &Modifiers, pressed: bool) {
        let mode = *self.term.lock().mode();
        let cell = self.screen_cell(position);
        if let Some(bytes) = mouse_report(button, cell, modifiers, pressed, mode) {
            self.write(bytes);
        }
    }

    fn on_any_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        if !self.program_wants_mouse(&event.modifiers) {
            return;
        }
        let Some(button) = button_code(event.button) else { return };
        self.term.lock().selection = None;
        self.mouse_held = Some(event.button);
        self.mouse_cell = Some(self.screen_cell(event.position));
        self.report_mouse(button, event.position, &event.modifiers, true);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_any_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(held) = self.mouse_held.take() else { return };
        if let Some(button) = button_code(held) {
            self.report_mouse(button, event.position, &event.modifiers, false);
        }
        self.mouse_cell = None;
        cx.stop_propagation();
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        if self.mouse_held.is_some() {
            return;
        }
        // ⌘-click on a link opens it rather than selecting.
        if event.modifiers.secondary()
            && let Some(link) = self.link_at(event.position)
        {
            self.link = None;
            match link.target {
                LinkTarget::Url(url) => cx.open_url(&url),
                LinkTarget::File(path, line, column) => cx.emit(TerminalEvent::OpenFile(path, line, column)),
            }
            cx.notify();
            return;
        }
        let (point, side) = self.grid_point(event.position);
        let kind = match event.click_count {
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        self.term.lock().selection = Some(Selection::new(kind, point, side));
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.pressed_button.is_none() {
            self.update_link(event.position, &event.modifiers, cx);
        }
        let mode = *self.term.lock().mode();
        // Dragging (or just moving, if the program asked) is reported cell by cell.
        let report_motion = match self.mouse_held {
            Some(_) => mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION),
            None => mode.contains(TermMode::MOUSE_MOTION) && !event.modifiers.shift,
        };
        if report_motion {
            let cell = self.screen_cell(event.position);
            if self.mouse_cell != Some(cell) {
                self.mouse_cell = Some(cell);
                let button = self.mouse_held.and_then(button_code).unwrap_or(3);
                self.report_mouse(button + 32, event.position, &event.modifiers, true);
            }
            return;
        }
        if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            let (point, side) = self.grid_point(event.position);
            if let Some(selection) = self.term.lock().selection.as_mut() {
                selection.update(point, side);
            }
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        // A plain click (no drag) shouldn't leave an empty selection behind.
        let mut term = self.term.lock();
        if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
            term.selection = None;
        }
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        // Small trackpad moves add up instead of being rounded away.
        self.scroll_rest += match event.delta {
            ScrollDelta::Lines(delta) => delta.y * 3.,
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / f32::from(self.cell.1),
        };
        let lines = self.scroll_rest.trunc() as i32;
        self.scroll_rest -= lines as f32;
        if lines == 0 {
            return;
        }
        let mode = *self.term.lock().mode();
        // A program that took the mouse gets the wheel too.
        if self.program_wants_mouse(&event.modifiers) {
            let button = if lines > 0 { 64 } else { 65 };
            for _ in 0..lines.unsigned_abs() {
                self.report_mouse(button, event.position, &event.modifiers, true);
            }
            return;
        }
        // Full-screen programs (less, man, git log, vim) scroll with the arrow keys.
        if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            let arrow: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.write(arrow.repeat(lines.unsigned_abs() as usize));
            return;
        }
        self.term.lock().scroll_display(Scroll::Delta(lines));
        cx.notify();
    }
}

/// The button number terminals report (0 left, 1 middle, 2 right).
fn button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

/// A mouse event as a terminal program reads it: `button` (with 32 added for motion,
/// 64/65 for the wheel), the cell (column, line from the top-left), Shift/Alt/Ctrl, and
/// whether it's a press. SGR encoding when the program asked for it (any size of
/// screen), else the classic one (cells up to 223), UTF-8 encoded if asked.
fn mouse_report(
    button: u8,
    (column, line): (usize, usize),
    modifiers: &Modifiers,
    pressed: bool,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let mods = (modifiers.shift as u8) * 4 + (modifiers.alt as u8) * 8 + (modifiers.control as u8) * 16;
    let (x, y) = (column + 1, line + 1);
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{};{x};{y}{end}", button + mods).into_bytes());
    }
    // The classic encoding has no release per button: 3 means "released".
    let code = if pressed { button + mods } else { 3 + mods };
    let mut bytes = b"\x1b[M".to_vec();
    bytes.push(32 + code);
    for v in [x, y] {
        if mode.contains(TermMode::UTF8_MOUSE) {
            let c = char::from_u32(32 + v as u32)?;
            bytes.extend(c.to_string().as_bytes());
        } else {
            bytes.push(u8::try_from(32 + v).ok()?);
        }
    }
    Some(bytes)
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        self.sender.send(Msg::Shutdown).ok();
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Terminal")
            // Files dropped on the terminal are typed in, quoted for the shell, as Terminal does.
            .on_drop(cx.listener(|this, dropped: &gpui::ExternalPaths, _, _| {
                this.write(shell_words(dropped.paths()).into_bytes())
            }))
            .track_focus(&self.focus_handle)
            .size_full()
            .cursor(if self.link.is_some() { gpui::CursorStyle::PointingHand } else { gpui::CursorStyle::IBeam })
            // Letting go of ⌘ takes the underline away at once.
            .on_modifiers_changed(cx.listener(|this, event: &gpui::ModifiersChangedEvent, _, cx| {
                if !event.modifiers.secondary() && this.link.take().is_some() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::open_find))
            .on_action(cx.listener(Self::close_find))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.step_match(true, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.step_match(false, cx)))
            .on_key_down(cx.listener(Self::on_key_down))
            // A program that took the mouse gets every button first (capture runs before the
            // selection handlers below).
            .capture_any_mouse_down(cx.listener(Self::on_any_mouse_down))
            .capture_any_mouse_up(cx.listener(Self::on_any_mouse_up))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .relative()
            .child(TerminalElement { view: cx.entity() })
            .children(self.render_find(cx))
    }
}

impl TerminalView {
    /// The find field, quiet in the top right corner: what's typed, and how many it found.
    fn render_find(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let find = self.find.as_ref()?;
        let theme = cx.global::<Theme>();
        let empty = find.input.read(cx).text().is_empty();
        let count = match (find.current, find.matches.len()) {
            _ if empty => String::new(),
            (_, 0) => "No matches".into(),
            (Some(i), n) => format!("{} of {n}", i + 1),
            (None, n) => n.to_string(),
        };
        Some(
            div()
                .key_context("TerminalFind")
                .absolute()
                .top(px(6.))
                .right(px(12.))
                .w(px(280.))
                .h(px(28.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(crate::ui::R_CONTROL))
                .bg(theme.raised)
                .border_1()
                .border_color(theme.line_strong)
                .shadow_md()
                .font_family(cx.global::<Fonts>().ui.clone())
                .text_size(px(crate::ui::T_SM))
                // Clicks here stay here, not starting a selection in the output below.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex_1().min_w_0().child(find.input.clone()))
                .child(div().flex_none().text_color(theme.muted).child(count))
                .into_any_element(),
        )
    }
}

/// What a key press sends to the shell, as the escape sequences terminals use.
pub fn key_to_bytes(keystroke: &Keystroke, app_cursor: bool) -> Option<Vec<u8>> {
    let m = &keystroke.modifiers;
    if m.platform {
        return None; // Cmd shortcuts belong to the app
    }
    let modifier_code = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8;
    let csi = |final_byte: char| -> Vec<u8> {
        if modifier_code > 1 {
            format!("\x1b[1;{modifier_code}{final_byte}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if modifier_code > 1 { format!("\x1b[{code};{modifier_code}~") } else { format!("\x1b[{code}~") }.into_bytes()
    };
    let meta = |bytes: Vec<u8>| if m.alt { [b"\x1b".to_vec(), bytes].concat() } else { bytes };
    Some(match keystroke.key.as_str() {
        "enter" => meta(b"\r".to_vec()),
        "backspace" if m.control => b"\x08".to_vec(),
        "backspace" => meta(b"\x7f".to_vec()),
        "tab" if m.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "up" => csi('A'),
        "down" => csi('B'),
        "right" => csi('C'),
        "left" => csi('D'),
        "home" => csi('H'),
        "end" => csi('F'),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "delete" => tilde(3),
        "insert" => tilde(2),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if m.control => vec![0],
        key if m.control && key.chars().count() == 1 => {
            let c = key.chars().next()?.to_ascii_lowercase();
            let byte = match c {
                'a'..='z' => c as u8 - b'a' + 1,
                '@' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '7' => 0x1f,
                '?' | '8' => 0x7f,
                _ => return None,
            };
            meta(vec![byte])
        }
        // On a Mac, Option types characters on many keyboards (| [ ] { } ~ \ on French and
        // German ones): send what it types, as the Mac's own Terminal does.
        key if m.alt
            && cfg!(target_os = "macos")
            && keystroke.key_char.as_deref().is_some_and(|c| !c.is_empty() && c != key) =>
        {
            keystroke.key_char.as_ref()?.as_bytes().to_vec()
        }
        // Elsewhere, Alt + a key acts as Meta, as shells expect (Alt+B / Alt+F move by word).
        key if m.alt && key.chars().count() == 1 => {
            let c = if m.shift { key.to_uppercase() } else { key.to_string() };
            meta(c.into_bytes())
        }
        "space" => b" ".to_vec(),
        _ => keystroke.key_char.as_ref()?.as_bytes().to_vec(),
    })
}

/// The 256-color palette: the theme's 16 colors, then the standard cube and grays.
fn indexed_color(index: usize, theme: &Theme) -> Hsla {
    match index {
        0..16 => theme.ansi[index],
        16..232 => {
            let i = index - 16;
            let level = |v: usize| if v == 0 { 0. } else { (55. + 40. * v as f32) / 255. };
            gpui::Rgba { r: level(i / 36), g: level((i / 6) % 6), b: level(i % 6), a: 1. }.into()
        }
        232..256 => {
            let v = (8. + 10. * (index - 232) as f32) / 255.;
            gpui::Rgba { r: v, g: v, b: v, a: 1. }.into()
        }
        // The named colors programs can ask about (OSC 10/11/12) to pick a light or dark look.
        256 => theme.foreground,
        257 => theme.background,
        258 => theme.caret,
        _ => theme.foreground,
    }
}

fn resolve(color: Color, colors: &alacritty_terminal::term::color::Colors, theme: &Theme, background: bool) -> Hsla {
    match color {
        Color::Spec(rgb) => {
            gpui::Rgba { r: rgb.r as f32 / 255., g: rgb.g as f32 / 255., b: rgb.b as f32 / 255., a: 1. }.into()
        }
        Color::Indexed(i) => colors[i as usize]
            .map(|rgb| resolve(Color::Spec(rgb), colors, theme, background))
            .unwrap_or_else(|| indexed_color(i as usize, theme)),
        Color::Named(named) => match named {
            NamedColor::Foreground | NamedColor::BrightForeground => theme.foreground,
            NamedColor::Background => theme.background,
            NamedColor::DimForeground => theme.muted,
            NamedColor::Cursor => theme.caret,
            other => {
                let i = other as usize;
                if i < 16 {
                    indexed_color(i, theme)
                } else if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) {
                    indexed_color(i - NamedColor::DimBlack as usize, theme).opacity(0.7)
                } else if background {
                    theme.background
                } else {
                    theme.foreground
                }
            }
        },
    }
}

/// Cells waiting to be drawn as one piece of text: text, first column, line, font, color, underline.
/// Cells with the same look, side by side, drawn as one piece of text.
struct Pending {
    text: String,
    column: usize,
    line: i32,
    font: Font,
    color: Hsla,
    underline: Option<UnderlineStyle>,
    /// How many cells it takes (its chars, all plain ASCII while `ascii`).
    cells: usize,
    ascii: bool,
}

type PendingRun = Option<Pending>;

impl Pending {
    fn new(column: usize, line: i32, font: Font, color: Hsla, underline: Option<UnderlineStyle>) -> Self {
        Pending { text: String::new(), column, line, font, color, underline, cells: 0, ascii: true }
    }

    /// Whether the cell at (`line`, `column`) goes on the end of it: right after it, plain
    /// ASCII one cell wide (the code font draws those exactly a cell wide; other characters
    /// can be a little wider or narrower, so each sits on its own cell instead of pushing the
    /// rest along), and looking the same. A space only has to be right after it (its colour
    /// and font don't show) and not underlined differently.
    fn takes(&self, c: char, line: i32, column: usize, font: &Font, color: Hsla, underline: Option<UnderlineStyle>) -> bool {
        let next = self.line == line && self.column + self.cells == column && self.ascii && c.is_ascii();
        if c == ' ' {
            return next && self.underline == underline;
        }
        next && self.font == *font && self.color == color && self.underline == underline
    }

    fn push(&mut self, c: char, zero_width: Option<&[char]>) {
        self.text.push(c);
        self.cells += 1;
        self.ascii &= c.is_ascii();
        if let Some(extra) = zero_width {
            self.text.extend(extra);
            self.ascii = false;
        }
    }
}

struct TerminalElement {
    view: Entity<TerminalView>,
}

struct Prepaint {
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    runs: Vec<(ShapedLine, Point<Pixels>)>,
    cursor: Option<Bounds<Pixels>>,
    cursor_hollow: bool,
    line_height: Pixels,
    /// Under the link the mouse is over while ⌘ is held.
    link_underline: Option<Bounds<Pixels>>,
    /// Behind what ⌘F found on screen; true for the current one.
    matches: Vec<(Bounds<Pixels>, bool)>,
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let theme = cx.global::<Theme>().clone();
        let family = cx.global::<Fonts>().code.clone();
        let font_size = px((cx.global::<crate::settings::Settings>().font_size - FONT_SIZE_BELOW_EDITOR).max(9.));
        // No ligatures in the terminal, whatever the editor does: output means what it
        // shows character by character (`-->` in a compiler's message isn't an arrow).
        let mut base = font(family);
        base.features = gpui::FontFeatures(Arc::new(vec![("calt".into(), 0), ("liga".into(), 0)]));
        let text_system = window.text_system().clone();
        let run = |len: usize, font: Font, color: Hsla| TextRun {
            len,
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let cell_width = text_system
            .shape_line("0".repeat(10).into(), font_size, &[run(10, base.clone(), theme.foreground)], None)
            .width
            / 10.;
        let line_height = (font_size * 1.35).round();
        let origin = point(bounds.left() + px(PADDING), bounds.top() + px(PADDING));
        let columns =
            (f32::from(bounds.size.width - px(PADDING * 2.)) / f32::from(cell_width)).floor().max(2.) as usize;
        let lines =
            (f32::from(bounds.size.height - px(PADDING * 2.)) / f32::from(line_height)).floor().max(1.) as usize;
        let focused = self.view.read(cx).focus_handle.is_focused(window);

        self.view.update(cx, |view, _| {
            view.origin = origin;
            view.resize(GridSize { columns, lines }, (cell_width, line_height));
        });

        let view = self.view.read(cx);
        let term = view.term.lock();
        let content = term.renderable_content();
        let selection = content.selection;
        let display_offset = content.display_offset as i32;
        let colors = content.colors;

        let mut backgrounds = Vec::new();
        let mut runs = Vec::new();
        // Consecutive cells with the same look are drawn as one piece of text.
        let mut pending: PendingRun = None;
        let mut flush = |pending: &mut PendingRun| {
            if let Some(Pending { text, column, line, font, color, underline, .. }) = pending.take() {
                // (Spaces at its end draw nothing, unless they're underlined.)
                let text = if underline.is_none() { text.trim_end_matches(' ').to_string() } else { text };
                if text.is_empty() {
                    return;
                }
                let mut r = run(text.len(), font, color);
                r.underline = underline;
                let shaped = text_system.shape_line(text.into(), font_size, &[r], None);
                let at = point(origin.x + cell_width * column as f32, origin.y + line_height * line as f32);
                runs.push((shaped, at));
            }
        };
        for indexed in content.display_iter {
            let cell = &indexed.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let line = indexed.point.line.0 + display_offset;
            let column = indexed.point.column.0;
            let selected = selection.is_some_and(|s| s.contains(indexed.point));
            let mut fg = resolve(cell.fg, colors, &theme, false);
            let mut bg = resolve(cell.bg, colors, &theme, true);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = fg.opacity(0.7);
            }
            let wide = if cell.flags.contains(Flags::WIDE_CHAR) { 2. } else { 1. };
            let cell_bounds = Bounds::new(
                point(origin.x + cell_width * column as f32, origin.y + line_height * line as f32),
                size(cell_width * wide, line_height),
            );
            if selected {
                backgrounds.push((cell_bounds, theme.selection));
            } else if bg != theme.background {
                backgrounds.push((cell_bounds, bg));
            }
            let c = cell.c;
            let blank = c == ' ' || c == '\t' || cell.flags.contains(Flags::HIDDEN);
            let mut cell_font = base.clone();
            if cell.flags.contains(Flags::BOLD) {
                cell_font.weight = FontWeight::BOLD;
            }
            if cell.flags.contains(Flags::ITALIC) {
                cell_font.style = FontStyle::Italic;
            }
            let underline = cell.flags.intersects(Flags::ALL_UNDERLINES).then_some(UnderlineStyle {
                color: Some(fg),
                thickness: px(1.),
                wavy: cell.flags.contains(Flags::UNDERCURL),
            });
            // A blank goes on the text before it (one piece for a line of words, not one per
            // word), or nowhere.
            let shown = if blank { ' ' } else { c };
            let continues =
                wide == 1. && pending.as_ref().is_some_and(|p| p.takes(shown, line, column, &cell_font, fg, underline));
            if blank && !continues {
                flush(&mut pending);
                continue;
            }
            if !continues {
                flush(&mut pending);
                pending = Some(Pending::new(column, line, cell_font, fg, underline));
            }
            if let Some(p) = pending.as_mut() {
                p.push(shown, if blank { None } else { cell.zerowidth() });
            }
            if wide > 1. {
                flush(&mut pending);
            }
        }
        flush(&mut pending);

        let cursor_point = content.cursor.point;
        let cursor_line = cursor_point.line.0 + display_offset;
        let cursor = (content.cursor.shape != CursorShape::Hidden
            && cursor_line >= 0
            && (cursor_line as usize) < lines)
            .then(|| {
                let at = point(
                    origin.x + cell_width * cursor_point.column.0 as f32,
                    origin.y + line_height * cursor_line as f32,
                );
                match content.cursor.shape {
                    CursorShape::Beam => Bounds::new(at, size(px(2.), line_height)),
                    CursorShape::Underline => {
                        Bounds::new(point(at.x, at.y + line_height - px(2.)), size(cell_width, px(2.)))
                    }
                    _ => Bounds::new(at, size(cell_width, line_height)),
                }
            });
        let link_underline = view.link.as_ref().and_then(|link| {
            let row = link.line + display_offset;
            (row >= 0 && (row as usize) < lines).then(|| {
                let left = origin.x + cell_width * link.columns.start as f32;
                let top = origin.y + line_height * row as f32 + line_height - px(3.);
                Bounds::new(point(left, top), size(cell_width * link.columns.len() as f32, px(1.)))
            })
        });
        let matches = view.find.as_ref().map_or(Vec::new(), |find| {
            find.matches
                .iter()
                .enumerate()
                .filter_map(|(i, (line, columns))| {
                    let row = line + display_offset;
                    (row >= 0 && (row as usize) < lines).then(|| {
                        let at =
                            point(origin.x + cell_width * columns.start as f32, origin.y + line_height * row as f32);
                        (Bounds::new(at, size(cell_width * columns.len() as f32, line_height)), find.current == Some(i))
                    })
                })
                .collect()
        });
        Prepaint { backgrounds, runs, cursor, cursor_hollow: !focused, line_height, link_underline, matches }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let theme = cx.global::<Theme>().clone();
        window.paint_quad(fill(bounds, theme.background));
        for (rect, color) in &prepaint.backgrounds {
            window.paint_quad(fill(*rect, *color));
        }
        for (rect, current) in &prepaint.matches {
            let quad = fill(*rect, theme.find_match).corner_radii(px(3.));
            window.paint_quad(if *current { quad.border_widths(px(1.)).border_color(theme.caret) } else { quad });
        }
        if let Some(cursor) = prepaint.cursor {
            let quad = if prepaint.cursor_hollow {
                fill(cursor, gpui::transparent_black()).border_widths(px(1.)).border_color(theme.caret)
            } else {
                fill(cursor, theme.caret.opacity(0.85))
            };
            window.paint_quad(quad);
        }
        for (line, origin) in &prepaint.runs {
            line.paint(*origin, prepaint.line_height, window, cx).ok();
        }
        if let Some(underline) = prepaint.link_underline {
            window.paint_quad(fill(underline, theme.foreground.opacity(0.8)));
        }
    }
}

/// Paths as a shell reads them back: plain ones as they are, others in single quotes; each
/// followed by a space, ready for the next word.
/// The lines running `text` enters, one by one: tabs as spaces, control characters left out.
fn run_lines(text: &str) -> Vec<String> {
    let text = text.replace("\r\n", "\n");
    text.trim_end_matches('\n')
        .split(['\n', '\r'])
        .map(|line| line.replace('\t', "    ").chars().filter(|c| !c.is_control()).collect())
        .collect()
}

fn shell_words(paths: &[std::path::PathBuf]) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-+,:@%~".contains(c);
    paths
        .iter()
        .map(|path| {
            let path = path.display().to_string();
            if path.chars().all(plain) { format!("{path} ") } else { format!("'{}' ", path.replace('\'', r"'\''")) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line of words is one piece of text to draw, not one per word; what looks different
    /// (another colour, not plain ASCII, not right after) starts another.
    #[test]
    fn a_line_of_words_is_one_piece() {
        let font = gpui::font("Menlo");
        let (white, red) = (gpui::white(), gpui::red());
        let mut run = Pending::new(0, 0, font.clone(), white, None);
        for c in "cargo".chars() {
            run.push(c, None);
        }
        assert!(run.takes(' ', 0, 5, &font, red, None), "a space, whatever its colour");
        run.push(' ', None);
        assert!(run.takes('b', 0, 6, &font, white, None));
        assert!(!run.takes('b', 0, 6, &font, red, None), "another colour");
        assert!(!run.takes('b', 0, 7, &font, white, None), "not right after");
        assert!(!run.takes('b', 1, 6, &font, white, None), "another line");
        assert!(!run.takes('→', 0, 6, &font, white, None), "not plain ASCII: its own cell");
        let underline = Some(gpui::UnderlineStyle { thickness: gpui::px(1.), color: None, wavy: false });
        assert!(!run.takes(' ', 0, 6, &font, white, underline), "underlined differently");
        run.push('é', None);
        assert!(!run.takes('x', 0, 8, &font, white, None), "after what isn't ASCII");
    }

    #[test]
    fn the_command_typed_on_a_prompt() {
        assert_eq!(typed_command("ada@mac app % ls -la"), "ls -la");
        assert_eq!(typed_command("➜  app git:(main) ✗ ls src"), "ls src", "the prompt's words aren't the command");
        assert_eq!(typed_command("~/code/app (main) $ cargo build 2> err.log"), "cargo build 2> err.log");
        assert_eq!(typed_command("❯ npm test"), "npm test");
        assert_eq!(typed_command("➜  ~/code/app ./build.sh"), "./build.sh");
        assert_eq!(typed_command("ada@mac app % "), "");
        assert_eq!(typed_command("make"), "make");
        assert_eq!(typed_command("ada@mac app % ls src      at 10:42:13"), "ls src", "the prompt's right side");
        assert_eq!(program_of("FOO=1 ./scripts/build.sh -v"), "build.sh");
        assert_eq!(program_of(""), "");
    }

    #[test]
    fn dropped_paths_are_quoted_for_the_shell() {
        let paths = ["/Users/me/src/main.rs", "/Users/me/My Notes.md", "/tmp/it's.txt"].map(std::path::PathBuf::from);
        assert_eq!(shell_words(&paths), r"/Users/me/src/main.rs '/Users/me/My Notes.md' '/tmp/it'\''s.txt' ");
        // Run: each line entered, escapes gone.
        assert_eq!(run_lines("ls\r\necho \x1b[2Jhi\n\tdone\x04\n"), ["ls", "echo [2Jhi", "    done"]);
    }

    fn keys(s: &str) -> Option<Vec<u8>> {
        key_to_bytes(&Keystroke::parse(s).unwrap(), false)
    }

    #[test]
    fn finds_text_in_the_output() {
        let rows: Vec<(i32, Vec<char>)> = [(-1, "test a ... ok"), (0, "test b ... FAILED"), (1, "failed: 1")]
            .into_iter()
            .map(|(l, t)| (l, t.chars().collect()))
            .collect();
        // Lowercase finds both; with a capital, only that.
        assert_eq!(find_in_rows(&rows, "failed"), vec![(0, 11..17), (1, 0..6)]);
        assert_eq!(find_in_rows(&rows, "FAILED"), vec![(0, 11..17)]);
        assert_eq!(find_in_rows(&rows, "test"), vec![(-1, 0..4), (0, 0..4)]);
        assert!(find_in_rows(&rows, "").is_empty());
    }

    #[test]
    fn mouse_events_are_encoded_as_programs_ask() {
        let none = Modifiers::default();
        let sgr = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        // Left press and release at the top-left cell.
        assert_eq!(mouse_report(0, (0, 0), &none, true, sgr).unwrap(), b"\x1b[<0;1;1M");
        assert_eq!(mouse_report(0, (0, 0), &none, false, sgr).unwrap(), b"\x1b[<0;1;1m");
        // Wheel down with Ctrl, far right: SGR has no size limit.
        let ctrl = Modifiers { control: true, ..Default::default() };
        assert_eq!(mouse_report(65, (299, 9), &ctrl, true, sgr).unwrap(), b"\x1b[<81;300;10M");
        // The classic encoding: offsets of 32, release as button 3, nothing past column 223.
        let classic = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(mouse_report(2, (4, 1), &none, true, classic).unwrap(), b"\x1b[M\x22\x25\x22");
        assert_eq!(mouse_report(2, (4, 1), &none, false, classic).unwrap(), b"\x1b[M\x23\x25\x22");
        assert!(mouse_report(0, (300, 0), &none, true, classic).is_none());
        // UTF-8 mode goes further.
        let utf8 = classic | TermMode::UTF8_MOUSE;
        // Column 301 is sent as the character 32 + 301.
        assert_eq!(mouse_report(0, (300, 0), &none, true, utf8).unwrap(), "\x1b[M \u{14d}!".as_bytes());
    }

    #[test]
    fn translates_keys_to_terminal_sequences() {
        assert_eq!(keys("enter"), Some(b"\r".to_vec()));
        assert_eq!(keys("ctrl-c"), Some(vec![3]));
        assert_eq!(keys("ctrl-w"), Some(vec![23]));
        assert_eq!(keys("alt-b"), Some(b"\x1bb".to_vec()));
        assert_eq!(keys("up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(keys("ctrl-left"), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(keys("shift-tab"), Some(b"\x1b[Z".to_vec()));
        assert_eq!(keys("cmd-c"), None);
        assert_eq!(key_to_bytes(&Keystroke::parse("up").unwrap(), true), Some(b"\x1bOA".to_vec()));
    }
}
