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

actions!(terminal, [Copy, Paste, Clear]);

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
        ]);
    } else {
        keys.extend([KeyBinding::new("ctrl-shift-c", Copy, ctx), KeyBinding::new("ctrl-shift-v", Paste, ctx)]);
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
    cx.bind_keys(keys);
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
}

pub struct TerminalView {
    focus_handle: FocusHandle,
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    size: GridSize,
    cell: (Pixels, Pixels),
    pub title: String,
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
    /// Whether the shell has started (its output went quiet after the first prompt).
    settled: bool,
    /// Whether the shell has written anything yet.
    spoke: bool,
    settle_task: Option<Task<()>>,
    _events: Task<()>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

/// A shell that's running, before its view exists.
pub struct Shell {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    events: mpsc::UnboundedReceiver<TermEvent>,
    size: GridSize,
}

impl Shell {
    /// Starts the user's shell in `cwd`.
    pub fn start(cwd: PathBuf) -> std::io::Result<Self> {
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty());
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
            working_directory: Some(cwd),
            drain_on_exit: false,
            env,
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let size = GridSize { columns: 80, lines: 24 };
        let window_size = WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 };
        let pty = tty::new(&options, window_size, 0)?;
        let (tx, events) = mpsc::unbounded();
        let listener = Listener(tx);
        let config = Config { scrolling_history: SCROLLBACK, ..Default::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Self { term, sender, events, size })
    }
}

impl TerminalView {
    pub fn new(shell: Shell, cx: &mut Context<Self>) -> Self {
        let Shell { term, sender, mut events, size } = shell;
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
            selecting: false,
            mouse_held: None,
            mouse_cell: None,
            scroll_rest: 0.,
            origin: Point::default(),
            queued: Vec::new(),
            settled: false,
            spoke: false,
            settle_task: None,
            _events: events,
        }
    }

    /// Types `command` into the shell and presses Return; in a shell still starting,
    /// once it has.
    pub fn run_command(&mut self, command: &str, cx: &mut Context<Self>) {
        if self.settled {
            return self.write(format!("{command}\r").into_bytes());
        }
        self.queued.push(command.to_string());
        // Before its first prompt, wait for it (but not forever: a silent shell gets it anyway).
        let wait = if self.spoke { SETTLE_QUIET } else { SETTLE_AT_MOST };
        self.settle_after(wait, cx);
    }

    /// The shell counts as started once its output has been quiet a moment.
    fn settle_after(&mut self, wait: std::time::Duration, cx: &mut Context<Self>) {
        self.settle_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, _| {
                this.settled = true;
                for command in std::mem::take(&mut this.queued) {
                    this.write(format!("{command}\r").into_bytes());
                }
            })
            .ok();
        }));
    }

    fn handle_event(&mut self, event: TermEvent, cx: &mut Context<Self>) {
        match event {
            TermEvent::Wakeup => {
                // Output while starting: the prompt is coming; wait for it to go quiet.
                if !self.settled {
                    self.spoke = true;
                    self.settle_after(SETTLE_QUIET, cx);
                }
                cx.notify()
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

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
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

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = key_to_bytes(&event.keystroke, app_cursor) {
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
        self.term.lock().mode().intersects(TermMode::MOUSE_MODE) && !modifiers.shift
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
            .track_focus(&self.focus_handle)
            .size_full()
            .cursor(gpui::CursorStyle::IBeam)
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::clear))
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
            .child(TerminalElement { view: cx.entity() })
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
type PendingRun = Option<(String, usize, i32, Font, Hsla, Option<UnderlineStyle>)>;

struct TerminalElement {
    view: Entity<TerminalView>,
}

struct Prepaint {
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    runs: Vec<(ShapedLine, Point<Pixels>)>,
    cursor: Option<Bounds<Pixels>>,
    cursor_hollow: bool,
    line_height: Pixels,
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
        let base = font(family);
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
            if let Some((text, column, line, font, color, underline)) = pending.take() {
                let len = text.len();
                let mut r = run(len, font, color);
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
            if c == ' ' || c == '\t' || cell.flags.contains(Flags::HIDDEN) {
                flush(&mut pending);
                continue;
            }
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
            // Only plain ASCII joins a run: the code font draws it exactly one cell wide. Other
            // characters (box drawing, arrows, symbols from a fallback font) can be a little
            // wider or narrower, so each sits on its own cell instead of pushing the rest along.
            let continues = c.is_ascii()
                && pending.as_ref().is_some_and(|(text, start, l, f, color, u)| {
                    *l == line
                        && *start + text.chars().count() == column
                        && text.is_ascii()
                        && *f == cell_font
                        && *color == fg
                        && *u == underline
                        && wide == 1.
                });
            if !continues {
                flush(&mut pending);
                pending = Some((String::new(), column, line, cell_font, fg, underline));
            }
            if let Some((text, ..)) = pending.as_mut() {
                text.push(c);
                if let Some(extra) = cell.zerowidth() {
                    text.extend(extra);
                }
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
        Prepaint { backgrounds, runs, cursor, cursor_hollow: !focused, line_height }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(s: &str) -> Option<Vec<u8>> {
        key_to_bytes(&Keystroke::parse(s).unwrap(), false)
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
