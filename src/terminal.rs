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
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, NoAction, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, ShapedLine, Style, Task, TextRun, UnderlineStyle, Window, actions, div, fill, font, point,
    prelude::*, px, relative, size,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

actions!(terminal, [Copy, Paste, Clear]);

const FONT_SIZE: f32 = 13.;
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
    /// Scrolling smaller than a line, kept for the next event.
    scroll_rest: f32,
    /// Where the grid was drawn last frame, for mouse selection.
    origin: Point<Pixels>,
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
            scroll_rest: 0.,
            origin: Point::default(),
            _events: events,
        }
    }

    fn handle_event(&mut self, event: TermEvent, cx: &mut Context<Self>) {
        match event {
            TermEvent::Wakeup | TermEvent::CursorBlinkingChange | TermEvent::MouseCursorDirty => cx.notify(),
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

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
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
        let font_size = px(FONT_SIZE);
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
            let continues = pending.as_ref().is_some_and(|(text, start, l, f, color, u)| {
                *l == line
                    && *start + text.chars().count() == column
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
