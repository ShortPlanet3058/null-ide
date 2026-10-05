use crate::editor::{Editor, Layout, ScrollbarLayout};
use crate::fonts::Fonts;
use crate::highlight::{Span, spans_in};
use crate::settings::Settings;
use crate::theme::{Syntax, Theme};
use crate::wrap::{Row, WrapMap};
use gpui::{
    App, Bounds, ContentMask, Element, ElementId, ElementInputHandler, Entity, Focusable, Font, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, ShapedLine, StrikethroughStyle, Style, TextRun,
    UnderlineStyle, Window, fill, font, point, px, relative, size,
};
use lsp_types::DiagnosticSeverity;
use std::ops::Range;
use std::time::{Duration, Instant};

/// How long the caret takes to glide to a new position.
const GLIDE: Duration = Duration::from_millis(90);
/// The caret stays solid while you work and only starts blinking after this.
const BLINK_DELAY: Duration = Duration::from_millis(500);
/// …and blinks this long; then it stays lit and nothing is drawn until something happens
/// (each blink's fades are frames drawn, which kept an idle window busy).
const BLINK_FOR: Duration = Duration::from_secs(15);
const TOP_PADDING: f32 = 8.;
const TEXT_PADDING: f32 = 8.;
pub(crate) const GUTTER_PADDING: f32 = 16.;
/// Width of the scrollbar along the right edge.
const BAR: f32 = 10.;
/// Room in the gutter, after the line numbers, for the fold chevrons.
pub(crate) const FOLD_SPACE: f32 = 12.;

/// One row of text as drawn last frame.
pub struct RowLayout {
    pub row: Row,
    /// The row's text (part of a line when it wraps).
    pub text: String,
    pub shaped: ShapedLine,
    /// Where the text starts: past the indent on a continuation row.
    pub x: Pixels,
    /// What's shown that isn't in `text`: tabs drawn as spaces, and hints.
    pub gaps: Vec<Gap>,
}

/// Text shown in a row that isn't in it, at a byte of the row's text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    pub byte: usize,
    /// Bytes shown beyond the text's own.
    pub extra: usize,
    /// A hint, shown before the byte's character; else the byte is a tab, drawn wider.
    pub hint: bool,
}

impl RowLayout {
    /// Where a byte of `text` is in the text as shown.
    /// A caret before a hinted character sits before the hint.
    pub fn shown_byte(&self, byte: usize) -> usize {
        byte + self.gaps.iter().take_while(|g| g.byte < byte).map(|g| g.extra).sum::<usize>()
    }

    /// The byte of `text` shown at `shown`; inside a tab's spaces, its nearer edge;
    /// on a hint, the character it stands before.
    pub fn text_byte(&self, shown: usize) -> usize {
        let mut gained = 0;
        for gap in &self.gaps {
            let start = gap.byte + gained;
            if shown <= start {
                break;
            }
            if shown <= start + gap.extra {
                return if gap.hint || shown - start <= gap.extra / 2 { gap.byte } else { gap.byte + 1 };
            }
            gained += gap.extra;
        }
        shown - gained
    }
}

/// Tabs drawn as spaces up to the next tab stop: the text as shown, the runs stretched
/// to match, and each tab's byte with the bytes it gained.
fn expand_tabs(text: &str, runs: &[TextRun]) -> (String, Vec<TextRun>, Vec<(usize, usize)>) {
    if !text.contains('\t') {
        return (text.to_string(), runs.to_vec(), Vec::new());
    }
    let mut shown = String::with_capacity(text.len() + 8);
    let mut tabs = Vec::new();
    let mut col = 0;
    for (byte, c) in text.char_indices() {
        let width = crate::wrap::char_columns(c, col);
        if c == '\t' {
            shown.extend(std::iter::repeat_n(' ', width));
            tabs.push((byte, width - 1));
        } else {
            shown.push(c);
        }
        col += width;
    }
    let mut start = 0;
    let runs = runs
        .iter()
        .map(|run| {
            let end = start + run.len;
            let gained: usize = tabs.iter().filter(|(b, _)| (start..end).contains(b)).map(|(_, g)| g).sum();
            start = end;
            TextRun { len: run.len + gained, ..run.clone() }
        })
        .collect();
    (shown, runs, tabs)
}

/// Puts hints into a row's text, each before the byte it belongs to: the text as
/// shown, runs to match (the hints in `hint_run`'s style), and each hint's (byte, length).
fn insert_hints(
    text: &str,
    runs: &[TextRun],
    hints: &[(usize, String)],
    hint_run: impl Fn(usize) -> TextRun,
) -> (String, Vec<TextRun>, Vec<(usize, usize)>) {
    if hints.is_empty() {
        return (text.to_string(), runs.to_vec(), Vec::new());
    }
    let mut shown = String::with_capacity(text.len() + 16);
    let mut out = Vec::with_capacity(runs.len() + hints.len() * 2);
    let mut placed = Vec::with_capacity(hints.len());
    let mut hints = hints.iter().peekable();
    let mut at = 0;
    for run in runs {
        let end = at + run.len;
        let mut from = at;
        // Hints inside this run (or at its start) split it.
        while let Some((byte, hint)) = hints.next_if(|(b, _)| *b < end || (*b == end && end == text.len())) {
            let byte = (*byte).clamp(from, end);
            if byte > from {
                out.push(TextRun { len: byte - from, ..run.clone() });
                shown.push_str(&text[from..byte]);
            }
            out.push(hint_run(hint.len()));
            shown.push_str(hint);
            placed.push((byte, hint.len()));
            from = byte;
        }
        if end > from {
            out.push(TextRun { len: end - from, ..run.clone() });
            shown.push_str(&text[from..end]);
        }
        at = end;
    }
    // Hints past the end of the text (or a row without runs) go at the end.
    for (_, hint) in hints {
        out.push(hint_run(hint.len()));
        shown.push_str(hint);
        placed.push((text.len(), hint.len()));
    }
    (shown, out, placed)
}

/// The gaps of a row from its hints (byte, length in the text) and its tabs (byte in
/// the text as shown with the hints, bytes gained).
fn gaps_of(hints: &[(usize, usize)], tabs: &[(usize, usize)]) -> Vec<Gap> {
    let mut gaps: Vec<Gap> = hints.iter().map(|&(byte, extra)| Gap { byte, extra, hint: true }).collect();
    for &(shown, extra) in tabs {
        // Back to the text's own bytes: take off the hints shown before it. A hint
        // starts at its byte plus the hints before it.
        let mut hinted = 0;
        let mut before = 0;
        for &(byte, len) in hints {
            if byte + hinted < shown {
                before += len;
            }
            hinted += len;
        }
        gaps.push(Gap { byte: shown - before, extra, hint: false });
    }
    // At the same byte, the hint comes first: it's shown before the tab.
    gaps.sort_by_key(|g| (g.byte, !g.hint));
    gaps
}

/// The row a (line, column) is on, and its x position from the text's left edge.
pub fn position(
    rows: &[RowLayout],
    first_row: usize,
    wrap: &WrapMap,
    buffer: &crate::buffer::Buffer,
    char_width: Pixels,
    line: usize,
    col: usize,
) -> (usize, Pixels) {
    let (row, display_col) = wrap.to_display(line, col, buffer);
    match row.checked_sub(first_row).and_then(|i| rows.get(i)) {
        // On the row, in the part of it built (a long line is built only around the view).
        Some(r)
            if r.row.line == line
                && r.row.block.is_none()
                && r.row.cols.start <= col
                && (col < r.row.cols.end || (col == r.row.cols.end && r.row.last)) =>
        {
            let col = col - r.row.cols.start;
            (row, r.x + r.shaped.x_for_index(r.shown_byte(byte_of_column(&r.text, col))))
        }
        _ => (row, char_width * display_col as f32),
    }
}

/// How many lines of enclosing blocks can be pinned at the top.
const MAX_STICKY: usize = 3;

/// Columns of room either side of the view on a long line (see `wrap::LONG_LINE`), so
/// scrolling a little shows text at once.
const LONG_LINE_ROOM: usize = 256;

/// The first lines of the blocks the view is inside (started above the top line, go
/// on below it), outermost first, at most `max`, innermost kept. `line_at(n)` is the line
/// on the nth row from the top; pinned lines cover rows, so the top line is the first
/// one they leave showing.
fn sticky_lines(
    mut blocks_around: impl FnMut(usize) -> Vec<Range<usize>>,
    line_at: impl Fn(usize) -> Option<usize>,
    max: usize,
) -> Vec<usize> {
    let mut covered = 0;
    loop {
        let Some(top) = line_at(covered) else { return Vec::new() };
        let mut starts: Vec<usize> =
            blocks_around(top).into_iter().filter(|r| r.start < top && top < r.end).map(|r| r.start).collect();
        starts.sort_unstable();
        starts.dedup();
        let keep = starts.len().saturating_sub(max);
        starts.drain(..keep);
        if starts.len() <= covered {
            return starts;
        }
        covered = starts.len();
    }
}

/// What a folded line shows after its text.
pub const FOLDED: &str = " ⋯";
/// Space between a line's end and who last changed it.
const BLAME_GAP: &str = "      ";

/// Draws an [`Editor`]: gutter, current line, selection, text and caret.
pub struct EditorElement {
    editor: Entity<Editor>,
}

impl EditorElement {
    pub fn new(editor: Entity<Editor>) -> Self {
        Self { editor }
    }
}

pub struct Prepaint {
    text_bounds: Bounds<Pixels>,
    line_height: Pixels,
    current_line: Option<Bounds<Pixels>>,
    /// The debugger's breakpoints: a dot each, in the gutter (a ring for one with a condition).
    breakpoint_dots: Vec<(Bounds<Pixels>, bool)>,
    /// The line the debugger stopped on: a band across it, and a mark in the gutter.
    execution: Option<(Bounds<Pixels>, Bounds<Pixels>)>,
    /// Faint lines down the indentation, one per level.
    indent_guides: Vec<Bounds<Pixels>>,
    /// The line at the project's line length.
    line_guide: Option<Bounds<Pixels>>,
    /// The first lines of the blocks the view is inside, pinned at the top: the band
    /// behind them, and each one's text and line number.
    sticky: Option<(Bounds<Pixels>, Vec<(ShapedLine, Point<Pixels>, ShapedLine, Point<Pixels>)>)>,
    numbers: Vec<(ShapedLine, Point<Pixels>)>,
    /// Fold chevrons: where, and whether folded (pointing right) or open (down).
    chevrons: Vec<(Bounds<Pixels>, bool)>,
    lines: Vec<(ShapedLine, Point<Pixels>)>,
    selection: Vec<Bounds<Pixels>>,
    matches: Vec<(Bounds<Pixels>, bool)>,
    /// Other uses of the symbol at the caret.
    symbol_marks: Vec<Bounds<Pixels>>,
    link: Vec<Bounds<Pixels>>,
    git_marks: Vec<(Bounds<Pixels>, Hsla)>,
    assist_band: Option<Bounds<Pixels>>,
    ai_tints: Vec<(Bounds<Pixels>, Hsla)>,
    scroll_thumb: Option<(Bounds<Pixels>, f32)>,
    scroll_marks: Vec<(Bounds<Pixels>, Hsla)>,
    bracket_boxes: Vec<Bounds<Pixels>>,
    marked: Vec<Bounds<Pixels>>,
    caret: Option<(Bounds<Pixels>, f32)>,
    /// The other cursors, drawn without the glide.
    extra_carets: Vec<Bounds<Pixels>>,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

fn run(len: usize, font: &Font, color: gpui::Hsla) -> TextRun {
    TextRun { len, font: font.clone(), color, background_color: None, underline: None, strikethrough: None }
}

/// Text runs covering exactly `text`, colored by the highlight spans that
/// fall inside it. `line_start` is the line's byte offset in the document.
fn runs_for(
    text: &str,
    line_start: usize,
    spans: &[Span],
    underlines: &[(Range<usize>, Hsla)],
    theme: &Theme,
    font: &Font,
) -> Vec<TextRun> {
    let colored: Vec<Span> = spans_in(spans, line_start..line_start + text.len()).collect();
    // Cut the line wherever a color or an underline starts or ends.
    let mut cuts: Vec<usize> = vec![0, text.len()];
    cuts.extend(colored.iter().flat_map(|(r, _)| [r.start, r.end]));
    cuts.extend(underlines.iter().flat_map(|(r, _)| [r.start, r.end]));
    cuts.retain(|&c| c <= text.len() && text.is_char_boundary(c));
    cuts.sort_unstable();
    cuts.dedup();
    // Spans come in order, so one walk along them finds each piece's colour (a search per
    // piece took minutes on a minified file's one long line).
    let mut next = 0;
    cuts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            while colored.get(next).is_some_and(|(r, _)| r.end <= a) {
                next += 1;
            }
            let syntax =
                colored.get(next).filter(|(r, _)| r.start <= a && b <= r.end).map_or(Syntax::Plain, |(_, s)| *s);
            let mut run = run(b - a, font, theme.syntax(syntax));
            if let Some((_, color)) = underlines.iter().find(|(r, _)| r.start <= a && b <= r.end) {
                run.underline = Some(UnderlineStyle { color: Some(*color), thickness: px(1.), wavy: true });
            }
            run
        })
        .collect()
}

/// A column's byte in `text`, line `line` of `buffer`, found through the rope: walking
/// the text to it, for every row, was most of a frame far along a long wrapped line.
fn byte_in_line(buffer: &crate::buffer::Buffer, text: &str, line: usize, col: usize) -> usize {
    let rope = buffer.rope();
    let col = col.min(buffer.line_len(line));
    (rope.char_to_byte(rope.line_to_char(line) + col) - rope.line_to_byte(line)).min(text.len())
}

fn byte_of_column(text: &str, column: usize) -> usize {
    text.char_indices().nth(column).map_or(text.len(), |(byte, _)| byte)
}

fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

fn lerp(a: Pixels, b: Pixels, t: f32) -> Pixels {
    a + (b - a) * t
}

impl Element for EditorElement {
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
    ) -> (LayoutId, Self::RequestLayoutState) {
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
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let theme = cx.global::<Theme>().clone();
        let code_font = cx.global::<Fonts>().code.clone();
        self.editor.update(cx, |editor, cx| {
            let now = Instant::now();
            let focused = editor.focus_handle(cx).is_focused(window) && window.is_window_active();
            let font_size = editor.font_size;
            let line_height = editor.line_height();
            let lh = f32::from(line_height);
            let mut font = font(code_font.clone());
            if !cx.global::<Settings>().ligatures {
                // Both kinds: some fonts (Geist Mono) join characters through `liga` too.
                font.features = gpui::FontFeatures(std::sync::Arc::new(vec![("calt".into(), 0), ("liga".into(), 0)]));
            }
            let text_system = window.text_system().clone();
            let shape = |text: String, runs: &[TextRun]| text_system.shape_line(text.into(), font_size, runs, None);
            // A row of text, with its tabs drawn as spaces.
            let shape_row = |text: &str, runs: &[TextRun]| {
                let (shown, runs, tabs) = expand_tabs(text, runs);
                (shape(shown, &runs), gaps_of(&[], &tabs))
            };
            // Type hints: faint, on a soft background, in the code's own font.
            let hint_style =
                |len: usize| TextRun { background_color: Some(theme.hairline), ..run(len, &font, theme.faint) };
            // A row with its hints in, then `suffix` (a fold's ⋯, who changed the line).
            let shape_hinted = |text: &str,
                                runs: &[TextRun],
                                hints: &[(usize, String)],
                                suffix: Option<TextRun>,
                                suffix_text: &str| {
                let (mut shown, mut runs, placed) = insert_hints(text, runs, hints, hint_style);
                if let Some(run) = suffix {
                    shown.push_str(suffix_text);
                    runs.push(run);
                }
                let (shown, runs, tabs) = expand_tabs(&shown, &runs);
                (shape(shown, &runs), gaps_of(&placed, &tabs))
            };
            let show_hints = cx.global::<Settings>().inlay_hints;
            if show_hints {
                editor.ensure_hints(cx);
            }

            let char_width = shape("0".repeat(10), &[run(10, &font, theme.foreground)]).width / 10.;
            let total_lines = editor.buffer.len_lines();
            let digits = total_lines.to_string().len().max(3);
            let gutter_width = char_width * digits as f32 + px(GUTTER_PADDING * 2. + FOLD_SPACE);
            let text_bounds =
                Bounds::from_corners(point(bounds.left() + gutter_width, bounds.top()), bounds.bottom_right());
            let viewport_height = f32::from(bounds.size.height);
            let (caret_line, caret_col) = editor.caret_point();
            let text_width = f32::from(text_bounds.size.width) - TEXT_PADDING;
            let cw = f32::from(char_width);

            // Word wrap: rows as wide as the text area (clear of the scrollbar), in characters.
            let wrap_width = editor.wraps(cx).then(|| ((text_width - BAR - cw) / cw).floor().max(1.) as usize);
            editor.wrap.update(&editor.buffer, wrap_width, &editor.block_specs());
            let total_rows = editor.wrap.rows();
            let (caret_row, _) = editor.wrap.to_display(caret_line, caret_col, &editor.buffer);

            // Vertical scroll: follow the caret after keyboard moves, then glide toward the target.
            let max_y = total_rows.saturating_sub(1) as f32 * lh;
            // Dragging past an edge keeps scrolling, frame after frame.
            if editor.continue_drag(cx) {
                window.request_animation_frame();
            }
            // The view got shorter (a panel opening below): a caret that was in view stays in it.
            if let Some(was) = editor.viewport_height.replace(viewport_height)
                && viewport_height < was - 0.5
            {
                let y = caret_row as f32 * lh + TOP_PADDING;
                let was_seen = y >= editor.scroll.y && y + lh <= editor.scroll.y + was;
                if was_seen && y + lh > editor.scroll.y + viewport_height {
                    editor.scroll.y = y + lh - viewport_height;
                    editor.scroll.target_y = editor.scroll.y;
                }
            }
            if editor.autoscroll {
                // From the keyboard, keep a few lines of room; from the mouse, just reveal.
                let margin = if editor.reveal_only { 0. } else { (3. * lh).min(viewport_height / 3.) };
                let y = caret_row as f32 * lh + TOP_PADDING;
                if y - margin < editor.scroll.target_y {
                    editor.scroll.target_y = y - margin;
                } else if y + lh + margin > editor.scroll.target_y + viewport_height {
                    editor.scroll.target_y = y + lh + margin - viewport_height;
                }
            }
            editor.scroll.target_y = editor.scroll.target_y.clamp(0., max_y);
            editor.scroll.y = editor.scroll.y.clamp(0., max_y);
            let distance = editor.scroll.target_y - editor.scroll.y;
            if distance.abs() > 0.5 {
                let dt = editor.scroll.last_tick.map_or(1. / 60., |t| (now - t).as_secs_f32().min(0.05));
                editor.scroll.y += distance * (1. - (-dt * 18.).exp());
                editor.scroll.last_tick = Some(now);
                window.request_animation_frame();
            } else {
                editor.scroll.y = editor.scroll.target_y;
                editor.scroll.last_tick = None;
            }

            // Rows on screen, and the lines they belong to.
            let first = ((editor.scroll.y - TOP_PADDING) / lh).floor().max(0.) as usize;
            let count = (viewport_height / lh).ceil() as usize + 2;
            let visible = first.min(total_rows)..(first + count).min(total_rows);
            // A very long line (a minified file) is built only around what's on screen:
            // shaping all of it took longer than anyone would wait.
            let shown_cols = {
                let from = (editor.scroll.x / cw).floor().max(0.) as usize;
                let width = (text_width / cw).ceil() as usize;
                from.saturating_sub(LONG_LINE_ROOM)..from + width + LONG_LINE_ROOM
            };
            let rows: Vec<Row> = visible
                .clone()
                .map(|r| {
                    let mut row = editor.wrap.row(r, &editor.buffer);
                    if row.block.is_none() && row.cols.len() > crate::wrap::LONG_LINE {
                        let start = (row.cols.start + shown_cols.start).min(row.cols.end);
                        let end = (row.cols.start + shown_cols.end).min(row.cols.end);
                        row.last = row.last && end == row.cols.end;
                        row.indent += start - row.cols.start;
                        row.cols = start..end;
                    }
                    row
                })
                .collect();
            let lines_shown = match (rows.first(), rows.last()) {
                (Some(a), Some(b)) => a.line..b.line + 1,
                _ => 0..0,
            };
            let windowed = !editor.wrap.is_on()
                && rows.iter().any(|r| r.block.is_none() && r.cols.len() < editor.buffer.line_len(r.line));
            let texts: Vec<String> = lines_shown.clone().map(|l| editor.buffer.line_text(l)).collect();
            // Colour what the rows show: all of each line, but only the shown part of a long one.
            let shown_bytes = rows
                .iter()
                .filter(|r| r.block.is_none())
                .map(|r| {
                    let text = &texts[r.line - lines_shown.start];
                    let line = editor.buffer.line_to_byte(r.line);
                    let end = if r.last {
                        editor.buffer.line_to_byte(r.line + 1)
                    } else {
                        line + byte_in_line(&editor.buffer, text, r.line, r.cols.end)
                    };
                    line + byte_in_line(&editor.buffer, text, r.line, r.cols.start)..end
                })
                .reduce(|a, b| a.start.min(b.start)..a.end.max(b.end));
            if let Some(bytes) = shown_bytes {
                editor.highlight_bytes(bytes);
            }
            // The lines that can fold, needed only while the mouse is over the gutter.
            let over_gutter = editor.mouse_position.is_some_and(|p| {
                p.x >= bounds.left() && p.x < text_bounds.left() && p.y >= bounds.top() && p.y < bounds.bottom()
            });
            let foldable: std::collections::HashSet<usize> =
                if over_gutter { editor.foldable().iter().map(|r| r.start).collect() } else { Default::default() };

            // Errors and warnings get a wavy underline and color their line number.
            // Hints and notes only show in the hover card, to keep the code calm.
            let mut underlines: Vec<Vec<(Range<usize>, Hsla)>> = vec![Vec::new(); lines_shown.len()];
            let mut flagged: Vec<Option<Hsla>> = vec![None; lines_shown.len()];
            for problem in editor.problems(cx).iter() {
                let color = match problem.severity {
                    DiagnosticSeverity::ERROR => theme.error,
                    DiagnosticSeverity::WARNING => theme.warning,
                    _ => continue,
                };
                let (start_line, start_col) = editor.buffer.point(problem.range.start);
                let (end_line, end_col) = editor.buffer.point(problem.range.end);
                for line in start_line.max(lines_shown.start)..=end_line.min(lines_shown.end.saturating_sub(1)) {
                    let i = line - lines_shown.start;
                    let text = &texts[i];
                    let from = if line == start_line { start_col } else { 0 };
                    let mut to = if line == end_line { end_col } else { text.chars().count() };
                    if to <= from {
                        to = from + 1; // zero-width problems still get one character underlined
                    }
                    underlines[i].push((byte_of_column(text, from)..byte_of_column(text, to), color));
                    if line == start_line && flagged[i] != Some(theme.error) {
                        flagged[i] = Some(color);
                    }
                }
            }

            let show_blame = cx.global::<crate::settings::Settings>().line_blame;
            // A merge conflict's first line says how to resolve it, faintly.
            let conflicts = editor.conflicts();
            let resolve_tip = (!conflicts.is_empty())
                .then(|| crate::palette::shortcut(&crate::editor::QuickFix, cx))
                .flatten()
                .map(|keys| format!("{BLAME_GAP}{keys} to resolve"));
            let row_layouts: Vec<RowLayout> = rows
                .into_iter()
                .map(|row| {
                    // Lines a change removed: struck through, faint. Other blocks are drawn on top.
                    if let Some((block, i)) = row.block {
                        let text = match &editor.blocks[block].kind {
                            crate::editor::BlockKind::Removed(lines) => lines.get(i).cloned().unwrap_or_default(),
                            crate::editor::BlockKind::Writing(lines) => {
                                // The AI's new code, arriving: plain text on the "added" tint.
                                let text = lines.get(i).cloned().unwrap_or_default();
                                let (shaped, tabs) = shape_row(&text, &[run(text.len(), &font, theme.foreground)]);
                                return RowLayout { x: px(0.), row, text, shaped, gaps: tabs };
                            }
                            crate::editor::BlockKind::Ghost(lines) => {
                                // The rest of a ghost completion: faint, nothing struck.
                                let text = lines.get(i).cloned().unwrap_or_default();
                                let (shaped, tabs) = shape_row(&text, &[run(text.len(), &font, theme.faint)]);
                                return RowLayout { x: px(0.), row, text, shaped, gaps: tabs };
                            }
                            _ => String::new(),
                        };
                        // Struck through from the first character, not across the indentation.
                        let indent = text.len() - text.trim_start().len();
                        let mut struck = run(text.len() - indent, &font, theme.faint);
                        struck.strikethrough = Some(StrikethroughStyle { thickness: px(1.), color: Some(theme.faint) });
                        let runs: Vec<TextRun> =
                            if indent > 0 { vec![run(indent, &font, theme.faint), struck] } else { vec![struck] };
                        let (shaped, tabs) = shape_row(&text, &runs);
                        return RowLayout { x: px(0.), row, text, shaped, gaps: tabs };
                    }
                    let i = row.line - lines_shown.start;
                    let line_text = &texts[i];
                    let (b0, b1) = (
                        byte_in_line(&editor.buffer, line_text, row.line, row.cols.start),
                        byte_in_line(&editor.buffer, line_text, row.line, row.cols.end),
                    );
                    let text = line_text[b0..b1].to_string();
                    let row_underlines: Vec<(Range<usize>, Hsla)> = underlines[i]
                        .iter()
                        .filter_map(|(r, color)| {
                            let (start, end) = (r.start.max(b0), r.end.min(b1));
                            (start < end).then(|| (start - b0..end - b0, *color))
                        })
                        .collect();
                    let line_byte = editor.buffer.line_to_byte(row.line) + b0;
                    // A ghost completion's first line shows faintly right at the caret.
                    if row.last
                        && editor.ghost_line() == Some(row.line)
                        && let Some((ghost, _)) = editor.ghost_text()
                        && caret_col >= row.cols.start
                    {
                        let at = byte_of_column(&text, caret_col - row.cols.start);
                        let (before, after) = text.split_at(at);
                        let split = |r: &(Range<usize>, Hsla), from: usize, to: usize| {
                            let (start, end) = (r.0.start.max(from), r.0.end.min(to));
                            (start < end).then(|| (start - from..end - from, r.1))
                        };
                        let under_before: Vec<_> = row_underlines.iter().filter_map(|r| split(r, 0, at)).collect();
                        let under_after: Vec<_> =
                            row_underlines.iter().filter_map(|r| split(r, at, text.len())).collect();
                        let mut runs = runs_for(before, line_byte, &editor.spans, &under_before, &theme, &font);
                        runs.push(run(ghost.len(), &font, theme.faint));
                        runs.extend(runs_for(after, line_byte + at, &editor.spans, &under_after, &theme, &font));
                        let shown = format!("{before}{ghost}{after}");
                        let (shaped, tabs) = shape_row(&shown, &runs);
                        return RowLayout { x: char_width * row.indent as f32, row, text, shaped, gaps: tabs };
                    }
                    // The code the AI is rewriting fades while the new code appears.
                    if editor.ai_writing_lines().is_some_and(|l| l.contains(&row.line)) {
                        let (shaped, tabs) = shape_row(&text, &[run(text.len(), &font, theme.faint)]);
                        return RowLayout { x: char_width * row.indent as f32, row, text, shaped, gaps: tabs };
                    }
                    let runs = runs_for(&text, line_byte, &editor.spans, &row_underlines, &theme, &font);
                    // The row's type hints, at their bytes in its text.
                    let hints: Vec<(usize, String)> = if show_hints {
                        editor
                            .hints_on_line(row.line)
                            .filter(|(col, _)| {
                                (row.cols.start..row.cols.end).contains(col) || (row.last && *col == row.cols.end)
                            })
                            .map(|(col, label)| (byte_of_column(&text, col - row.cols.start), label.to_string()))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    // A folded line ends in "⋯", standing in for the lines it hides; the
                    // caret's line can end in who last changed it, faintly.
                    let blame = (row.last && show_blame && !editor.is_folded(row.line))
                        .then(|| editor.line_blame())
                        .flatten()
                        .filter(|(line, _)| *line == row.line)
                        .map(|(_, blame)| format!("{BLAME_GAP}{blame}"));
                    // While debugging: the values of what the line names, before anything else.
                    let values = row
                        .last
                        .then(|| editor.inline_values.iter().find(|(l, _)| *l == row.line))
                        .flatten()
                        .map(|(_, text)| format!("{BLAME_GAP}{text}"));
                    let tip = resolve_tip
                        .as_ref()
                        .filter(|_| row.last && conflicts.iter().any(|c| c.start == row.line))
                        .cloned();
                    let (suffix, suffix_text) = if row.last && editor.is_folded(row.line) {
                        (Some(run(FOLDED.len(), &font, theme.muted)), FOLDED.to_string())
                    } else if let Some(note) = values {
                        (Some(run(note.len(), &font, theme.muted)), note)
                    } else if let Some(note) = tip {
                        (Some(run(note.len(), &font, theme.faint)), note)
                    } else if let Some(note) = blame {
                        (Some(run(note.len(), &font, theme.faint)), note)
                    } else {
                        (None, String::new())
                    };
                    let (shaped, gaps) = shape_hinted(&text, &runs, &hints, suffix, &suffix_text);
                    RowLayout { x: char_width * row.indent as f32, row, text, shaped, gaps }
                })
                .collect();
            let wrap = &editor.wrap;
            let pos = |line: usize, col: usize| {
                position(&row_layouts, visible.start, wrap, &editor.buffer, char_width, line, col)
            };
            let row_of_line = |line: usize| wrap.first_row(line);

            // Horizontal scroll: keep the caret in view, never scroll past the longest visible
            // row. Nothing to scroll when lines wrap.
            let (_, caret_x) = pos(caret_line, caret_col);
            let caret_x = f32::from(caret_x);
            let scrolled_from = editor.scroll.x;
            if wrap.is_on() {
                editor.scroll.x = 0.;
            } else if editor.autoscroll {
                if caret_x < editor.scroll.x + cw * 2. {
                    editor.scroll.x = caret_x - cw * 4.;
                } else if caret_x > editor.scroll.x + text_width - cw * 4. {
                    editor.scroll.x = caret_x - text_width + cw * 8.;
                }
            }
            // The limit is the file's longest line, not the longest on screen: scrolling down
            // past a long line doesn't pull the view back.
            let widest = row_layouts
                .iter()
                .map(|r| f32::from(r.x + r.shaped.width))
                .fold(caret_x.max(editor.longest_line() as f32 * cw), f32::max);
            editor.scroll.x = editor.scroll.x.clamp(0., (widest + cw * 4. - text_width).max(0.));
            // A long line was built around where the view was: build it again where it is now.
            if editor.scroll.x != scrolled_from && windowed {
                window.request_animation_frame();
            }
            editor.autoscroll = false;
            editor.reveal_only = false;

            let origin = point(
                text_bounds.left() + px(TEXT_PADDING - editor.scroll.x),
                bounds.top() + px(TOP_PADDING - editor.scroll.y),
            );
            let row_top = |row: usize| origin.y + line_height * row as f32;
            // The rows a range of lines takes, clipped to the screen.
            let rows_of = |lines: Range<usize>| {
                let end =
                    if lines.end > lines.start { wrap.text_rows(lines.end - 1).end } else { row_of_line(lines.start) };
                row_of_line(lines.start).max(visible.start)..end.min(visible.end)
            };

            let selection_range = editor.selection.range();
            // While the Cmd+I card is open, the code it changes is tinted, with a bar in the gutter.
            let assist_band = editor.assist_target().map(rows_of).filter(|r| !r.is_empty()).map(|r| {
                Bounds::from_corners(
                    point(bounds.left() + gutter_width - px(9.), row_top(r.start)),
                    point(bounds.right(), row_top(r.end)),
                )
            });
            let caret_rows = wrap.text_rows(caret_line);
            // A change from the AI: its new lines tinted green, the removed ones red.
            let mut ai_tints: Vec<(Bounds<Pixels>, Hsla)> = editor
                .ai_added_lines()
                .into_iter()
                .map(&rows_of)
                .filter(|r| !r.is_empty())
                .map(|r| {
                    let rect = Bounds::from_corners(
                        point(bounds.left() + gutter_width - px(9.), row_top(r.start)),
                        point(bounds.right(), row_top(r.end)),
                    );
                    (rect, theme.git_added.opacity(0.13))
                })
                .collect();
            for (r, row) in row_layouts.iter().zip(visible.clone()) {
                let tint = match r.row.block.map(|(b, _)| &editor.blocks[b].kind) {
                    Some(crate::editor::BlockKind::Removed(_)) => Some(theme.git_deleted.opacity(0.13)),
                    Some(crate::editor::BlockKind::Writing(_)) => Some(theme.git_added.opacity(0.13)),
                    _ => None,
                };
                if let Some(tint) = tint {
                    let rect = Bounds::new(
                        point(bounds.left() + gutter_width - px(9.), row_top(row)),
                        size(bounds.size.width - gutter_width + px(9.), line_height),
                    );
                    ai_tints.push((rect, tint));
                }
            }
            // Merge conflicts: the current side tinted green, the incoming one blue, each
            // marker a little more; the common ancestor (diff3) and `=======` stay neutral.
            for c in conflicts.iter() {
                let band = |lines: Range<usize>, color: Hsla| {
                    let r = rows_of(lines);
                    (!r.is_empty()).then(|| {
                        let rect = Bounds::from_corners(
                            point(bounds.left() + gutter_width - px(9.), row_top(r.start)),
                            point(bounds.right(), row_top(r.end)),
                        );
                        (rect, color)
                    })
                };
                let (current, incoming, neutral) = (theme.git_added, theme.git_modified, theme.faint);
                ai_tints.extend(band(c.start..c.start + 1, current.opacity(0.2)));
                ai_tints.extend(band(c.current(), current.opacity(0.1)));
                ai_tints.extend(band(c.base.unwrap_or(c.middle)..c.middle + 1, neutral.opacity(0.1)));
                ai_tints.extend(band(c.incoming(), incoming.opacity(0.1)));
                ai_tints.extend(band(c.end..c.end + 1, incoming.opacity(0.2)));
            }
            let current_line = editor.selection.is_empty().then(|| {
                Bounds::new(
                    point(bounds.left(), row_top(caret_rows.start)),
                    size(bounds.size.width, line_height * caret_rows.len() as f32),
                )
            });
            // Breakpoints: a dot left of the line number, on the line's first row.
            let dot = px(8.);
            let breakpoint_dots: Vec<(Bounds<Pixels>, bool)> = editor
                .breakpoints
                .iter()
                .filter(|&&line| lines_shown.contains(&line))
                .filter_map(|&line| {
                    let rows = rows_of(line..line + 1);
                    (!rows.is_empty()).then(|| {
                        let y = row_top(rows.start) + (line_height - dot) / 2.;
                        let conditional = editor.breakpoint_conditions.iter().any(|(l, _)| *l == line);
                        (Bounds::new(point(bounds.left() + px(5.), y), size(dot, dot)), conditional)
                    })
                })
                .collect();
            let execution = editor.execution_line.and_then(|line| {
                let rows = rows_of(line..line + 1);
                (!rows.is_empty()).then(|| {
                    let height = line_height * rows.len() as f32;
                    let band = Bounds::new(point(bounds.left(), row_top(rows.start)), size(bounds.size.width, height));
                    let mark = Bounds::new(point(bounds.left(), row_top(rows.start)), size(px(3.), height));
                    (band, mark)
                })
            });

            // The project's line length: a faint line down the text at that column.
            let line_guide = editor.style.ruler.filter(|_| cx.global::<Settings>().line_guide).and_then(|width| {
                let x = text_bounds.left() + px(TEXT_PADDING) + char_width * width as f32 - px(editor.scroll.x);
                (x > text_bounds.left() && x < text_bounds.right())
                    .then(|| Bounds::new(point(x, text_bounds.top()), size(px(1.), text_bounds.size.height)))
            });
            // Indent guides: a faint line at each level a line is indented past. Blank lines
            // take the smaller indentation of the lines around them, so guides run through.
            let indent_guides: Vec<Bounds<Pixels>> = if cx.global::<Settings>().indent_guides {
                let unit = editor.style.indent.width().max(1);
                let indent_of = |text: &str| -> Option<usize> {
                    if text.trim().is_empty() {
                        return None;
                    }
                    let mut col = 0;
                    for c in text.chars().take_while(|c| *c == ' ' || *c == '\t') {
                        col += crate::wrap::char_columns(c, col);
                    }
                    Some(col)
                };
                let indents: Vec<Option<usize>> = texts.iter().map(|t| indent_of(t)).collect();
                let levels: Vec<usize> = (0..indents.len())
                    .map(|i| {
                        let cols = indents[i].unwrap_or_else(|| {
                            let before = indents[..i].iter().rev().find_map(|x| *x).unwrap_or(0);
                            let after = indents[i + 1..].iter().find_map(|x| *x).unwrap_or(0);
                            before.min(after)
                        });
                        cols / unit
                    })
                    .collect();
                row_layouts
                    .iter()
                    .zip(visible.clone())
                    .filter(|(r, _)| r.row.block.is_none())
                    .flat_map(|(r, row)| {
                        let level = levels.get(r.row.line - lines_shown.start).copied().unwrap_or(0);
                        (0..level).map(move |k| {
                            let x = origin.x + char_width * (k * unit) as f32;
                            Bounds::new(point(x.round(), row_top(row)), size(px(1.), line_height))
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };

            // Fold chevrons: on folded lines always, on lines that can fold while the mouse
            // is over the gutter. Quiet otherwise.
            let chevrons = row_layouts
                .iter()
                .zip(visible.clone())
                .filter(|(r, row)| editor.wrap.first_row(r.row.line) == *row && r.row.block.is_none())
                .filter_map(|(r, row)| {
                    let folded = editor.is_folded(r.row.line);
                    (folded || foldable.contains(&r.row.line)).then(|| {
                        let center = point(
                            bounds.left() + gutter_width - px(GUTTER_PADDING - 2. + FOLD_SPACE / 2.),
                            row_top(row) + line_height / 2.,
                        );
                        (Bounds::centered_at(center, size(px(10.), px(10.))), folded)
                    })
                })
                .collect();

            // Line numbers go on a line's first row only.
            let numbers = row_layouts
                .iter()
                .zip(visible.clone())
                .filter(|(r, row)| editor.wrap.first_row(r.row.line) == *row && r.row.block.is_none())
                .map(|(r, row)| {
                    let line = r.row.line;
                    let label = (line + 1).to_string();
                    let color = flagged[line - lines_shown.start].unwrap_or(if line == caret_line {
                        theme.muted
                    } else {
                        theme.faint
                    });
                    let shaped = shape(label.clone(), &[run(label.len(), &font, color)]);
                    let x = bounds.left() + gutter_width - px(GUTTER_PADDING + FOLD_SPACE) - shaped.width;
                    (shaped, point(x, row_top(row)))
                })
                .collect();

            // Rectangles covering a char range on the visible rows, one per row.
            let range_rects = |range: std::ops::Range<usize>| -> Vec<Bounds<Pixels>> {
                let mut rects = Vec::new();
                if range.is_empty() || row_layouts.is_empty() {
                    return rects;
                }
                let (start_line, start_col) = editor.buffer.point(range.start);
                let (end_line, end_col) = editor.buffer.point(range.end);
                let (start_row, x_start) = pos(start_line, start_col);
                let (end_row, x_end) = pos(end_line, end_col);
                for row in start_row.max(visible.start)..=end_row.min(visible.end - 1) {
                    let r = &row_layouts[row - visible.start];
                    if r.row.block.is_some() {
                        continue;
                    }
                    let x0 = if row == start_row { x_start } else { r.x };
                    let x1 = if row == end_row {
                        x_end
                    } else {
                        // A selected line break shows as a little extra past the end.
                        r.x + r.shaped.width + if r.row.last { char_width * 0.6 } else { px(0.) }
                    };
                    if x1 > x0 {
                        rects.push(Bounds::from_corners(
                            point(origin.x + x0, row_top(row)),
                            point(origin.x + x1, row_top(row) + line_height),
                        ));
                    }
                }
                rects
            };
            let mut selection = range_rects(selection_range.clone());
            for cursor in &editor.extra {
                selection.extend(range_rects(cursor.selection.range()));
            }
            // The bracket next to the caret and its partner get a thin outline.
            let bracket_boxes: Vec<Bounds<Pixels>> = editor
                .matching_brackets()
                .map(|(a, b)| [a, b])
                .into_iter()
                .flatten()
                .flat_map(|offset| range_rects(offset..offset + 1))
                .collect();
            // A thin line under a range of text.
            let underline = |range: Range<usize>, at: f32| -> Vec<Bounds<Pixels>> {
                range_rects(range)
                    .into_iter()
                    .map(|r| Bounds::new(point(r.left(), r.top() + line_height * at), size(r.size.width, px(1.))))
                    .collect()
            };
            // The word that Cmd/Ctrl+click would follow.
            let link = editor.link_word.clone().map(|r| underline(r, 0.82)).unwrap_or_default();

            // Changes since the last commit: a bar beside the line numbers, or a small
            // notch between lines where something was deleted.
            let marker_x = bounds.left() + gutter_width - px(9.);
            let git_marks: Vec<(Bounds<Pixels>, Hsla)> = editor
                .git_hunks
                .iter()
                .filter_map(|h| match h.change {
                    crate::git::Change::Deleted => {
                        let row = row_of_line(h.lines.start);
                        (row >= visible.start && row <= visible.end).then(|| {
                            (
                                Bounds::new(point(marker_x - px(2.), row_top(row) - px(1.5)), size(px(7.), px(3.))),
                                theme.git_deleted,
                            )
                        })
                    }
                    change => {
                        let color =
                            if change == crate::git::Change::Added { theme.git_added } else { theme.git_modified };
                        let rows = rows_of(h.lines.clone());
                        (!rows.is_empty()).then(|| {
                            (
                                Bounds::from_corners(
                                    point(marker_x, row_top(rows.start) + px(2.)),
                                    point(marker_x + px(3.), row_top(rows.end) - px(2.)),
                                ),
                                color,
                            )
                        })
                    }
                })
                .collect();

            // Search matches on screen; the current one is drawn with an outline.
            let mut matches = Vec::new();
            if let Some(search) = &editor.search {
                let first_char = editor.buffer.line_to_char(lines_shown.start);
                let last_char = editor.buffer.line_to_char(lines_shown.end);
                let first = search.matches.partition_point(|m| m.end <= first_char);
                for (i, m) in search.matches.iter().enumerate().skip(first) {
                    if m.start > last_char {
                        break;
                    }
                    let current = search.current == Some(i);
                    matches.extend(range_rects(m.clone()).into_iter().map(|r| (r, current)));
                }
            }

            // Other uses of the symbol at the caret, on screen.
            let symbol_marks: Vec<Bounds<Pixels>> = if !cx.global::<Settings>().symbol_marks {
                Vec::new()
            } else {
                let first_char = editor.buffer.line_to_char(lines_shown.start);
                let last_char = editor.buffer.line_to_char(lines_shown.end);
                editor
                    .symbol_marks()
                    .iter()
                    .filter(|r| r.end >= first_char && r.start <= last_char)
                    .flat_map(|r| range_rects(r.clone()))
                    .collect()
            };

            // Text being composed (an accent, or an input method) is underlined.
            let marked = editor.marked.clone().map(|r| underline(r, 0.85)).unwrap_or_default();

            let caret_height = line_height * 0.8;
            let caret_rect = |row: usize, x: Pixels| {
                Bounds::new(
                    point(origin.x + x - px(1.), row_top(row) + (line_height - caret_height) / 2.),
                    size(px(2.), caret_height),
                )
            };
            let extra_carets: Vec<Bounds<Pixels>> = editor
                .extra
                .iter()
                .map(|c| editor.buffer.point(c.selection.head))
                .map(|(line, col)| pos(line, col))
                .filter(|(row, _)| visible.contains(row))
                .map(|(row, x)| caret_rect(row, x))
                .collect();
            let lines = row_layouts
                .iter()
                .zip(visible.clone())
                .map(|(r, row)| (r.shaped.clone(), point(origin.x + r.x, row_top(row))))
                .collect();
            let scroll_row = |line: usize| row_of_line(line);
            let mut scroll_marks: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
            let scrollbar = (max_y > 0.).then(|| {
                let track = Bounds::from_corners(point(bounds.right() - px(BAR), bounds.top()), bounds.bottom_right());
                let track_h = f32::from(track.size.height);
                let thumb_h = (track_h * viewport_height / (viewport_height + max_y)).max(28.).min(track_h);
                let thumb_top = track.top() + px((editor.scroll.y / max_y) * (track_h - thumb_h));
                let thumb = Bounds::new(point(track.left(), thumb_top), size(px(BAR), px(thumb_h)));
                ScrollbarLayout { track, thumb, max_scroll: max_y }
            });
            // Marks on the scrollbar for errors, search matches and changed lines,
            // so they can be found in long files.
            if let Some(bar) = &scrollbar {
                let total = total_rows.max(1) as f32;
                let mark = |line: usize, x: f32, w: f32, color: Hsla| {
                    let y = bar.track.top() + bar.track.size.height * (scroll_row(line) as f32 / total);
                    (Bounds::new(point(bar.track.left() + px(x), y), size(px(w), px(2.))), color)
                };
                for hunk in &editor.git_hunks {
                    let color = match hunk.change {
                        crate::git::Change::Added => theme.git_added,
                        crate::git::Change::Modified => theme.git_modified,
                        crate::git::Change::Deleted => theme.git_deleted,
                    };
                    scroll_marks.push(mark(hunk.lines.start, 0., 3., color));
                }
                if let Some(search) = &editor.search {
                    for m in search.matches.iter().take(2000) {
                        scroll_marks.push(mark(editor.buffer.point(m.start).0, 3., 4., theme.caret.opacity(0.8)));
                    }
                }
                for problem in editor.problems(cx).iter() {
                    let color = match problem.severity {
                        DiagnosticSeverity::ERROR => theme.error,
                        DiagnosticSeverity::WARNING => theme.warning,
                        _ => continue,
                    };
                    scroll_marks.push(mark(editor.buffer.point(problem.range.start).0, 6., 4., color));
                }
            }
            let (_, caret_x) = pos(caret_line, caret_col);

            // Caret: glide toward its new position, then blink softly once idle.
            let target = point(caret_x, line_height * caret_row as f32);
            let motion = &mut editor.caret;
            if !motion.placed {
                (motion.from, motion.to, motion.visual, motion.placed) = (target, target, target, true);
            } else if target != motion.to {
                motion.from =
                    if (target.y - motion.visual.y).abs() > line_height * 40. { target } else { motion.visual };
                motion.to = target;
                motion.started = now;
            }
            let t = (now - motion.started).as_secs_f32() / GLIDE.as_secs_f32();
            let eased = ease_out(t);
            motion.visual = point(lerp(motion.from.x, motion.to.x, eased), lerp(motion.from.y, motion.to.y, eased));
            let gliding = t < 1.;
            if gliding {
                window.request_animation_frame();
            }
            let visual = motion.visual;

            let opacity = if !focused {
                0.35
            } else if gliding {
                1.
            } else {
                let idle = now - editor.last_activity;
                if idle < BLINK_DELAY {
                    editor.wake_after(BLINK_DELAY - idle, cx);
                    1.
                } else if idle >= BLINK_DELAY + BLINK_FOR {
                    // Resting: lit, and no more frames until the next keystroke or click.
                    1.
                } else {
                    let phase = ((idle - BLINK_DELAY).as_millis() % 1100) as u64;
                    let wake = |ms: u64, editor: &mut Editor, cx: &mut gpui::Context<Editor>| {
                        editor.wake_after(Duration::from_millis(ms), cx)
                    };
                    match phase {
                        0..450 => {
                            wake(450 - phase, editor, cx);
                            1.
                        }
                        450..600 => {
                            window.request_animation_frame();
                            1. - ease_out((phase - 450) as f32 / 150.)
                        }
                        600..950 => {
                            wake(950 - phase, editor, cx);
                            0.
                        }
                        _ => {
                            window.request_animation_frame();
                            ease_out((phase - 950) as f32 / 150.)
                        }
                    }
                }
            };
            let caret = Some((
                Bounds::new(
                    point(origin.x + visual.x - px(1.), origin.y + visual.y + (line_height - caret_height) / 2.),
                    size(px(2.), caret_height),
                ),
                opacity,
            ));

            let thumb_emphasis = if editor.scrollbar_dragging() {
                0.9
            } else if editor.over_scrollbar() {
                0.65
            } else {
                0.35
            };

            // Sticky scroll: the blocks the view has scrolled into keep their first line in
            // sight at the top, over the code.
            let mut sticky_rows: Vec<(Bounds<Pixels>, usize)> = Vec::new();
            let sticky = if cx.global::<Settings>().sticky_scroll && editor.scroll.y > 0. {
                let shown_rows: Vec<usize> = row_layouts.iter().map(|r| r.row.line).collect();
                // The top row can be half scrolled away: count from the first fully shown.
                let skip = usize::from(row_top(visible.start) < text_bounds.top());
                let lines =
                    sticky_lines(|line| editor.blocks_around(line), |n| shown_rows.get(n + skip).copied(), MAX_STICKY);
                (!lines.is_empty()).then(|| {
                    let top = bounds.top();
                    let band = Bounds::new(
                        point(bounds.left(), top),
                        size(bounds.size.width - px(BAR), line_height * lines.len() as f32),
                    );
                    let pinned = lines
                        .iter()
                        .enumerate()
                        .map(|(i, &line)| {
                            let y = top + line_height * i as f32;
                            let text = editor.buffer.line_text(line);
                            let line_byte = editor.buffer.line_to_byte(line);
                            let spans = editor.line_spans(line);
                            let runs = runs_for(&text, line_byte, &spans, &[], &theme, &font);
                            let (shaped, _) = shape_row(&text, &runs);
                            let label = (line + 1).to_string();
                            let number = shape(label.clone(), &[run(label.len(), &font, theme.faint)]);
                            let number_x =
                                bounds.left() + gutter_width - px(GUTTER_PADDING + FOLD_SPACE) - number.width;
                            sticky_rows
                                .push((Bounds::new(point(bounds.left(), y), size(band.size.width, line_height)), line));
                            (shaped, point(origin.x, y), number, point(number_x, y))
                        })
                        .collect();
                    (band, pinned)
                })
            } else {
                None
            };

            editor.layout = Some(Layout {
                text_origin: origin,
                text_bounds,
                bounds,
                line_height,
                char_width,
                first_row: visible.start,
                rows: row_layouts,
                sticky: sticky_rows,
                scrollbar: scrollbar.as_ref().map(|b| ScrollbarLayout {
                    track: b.track,
                    thumb: b.thumb,
                    max_scroll: b.max_scroll,
                }),
            });

            Prepaint {
                text_bounds,
                line_height,
                current_line,
                breakpoint_dots,
                execution,
                indent_guides,
                line_guide,
                sticky,
                numbers,
                chevrons,
                lines,
                selection,
                matches,
                symbol_marks,
                link,
                git_marks,
                assist_band,
                ai_tints,
                scroll_thumb: scrollbar.map(|b| (b.thumb, thumb_emphasis)),
                scroll_marks,
                bracket_boxes,
                marked,
                caret,
                extra_carets,
            }
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle(cx);
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.editor.clone()), cx);
        // A drag is followed everywhere in the window, not just over the editor.
        if self.editor.read(cx).is_dragging() {
            let editor = self.editor.clone();
            window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                if phase == gpui::DispatchPhase::Bubble && event.pressed_button == Some(gpui::MouseButton::Left) {
                    editor.update(cx, |editor, cx| editor.drag_to(event.position, cx));
                }
            });
        }

        let theme = cx.global::<Theme>().clone();
        let line_height = prepaint.line_height;
        window.paint_quad(fill(bounds, theme.background));
        if let Some(row) = prepaint.current_line {
            window.paint_quad(fill(row, theme.current_line));
        }
        if let Some((band, mark)) = prepaint.execution {
            window.paint_quad(fill(band, theme.warning.opacity(0.14)));
            window.paint_quad(fill(mark, theme.warning));
        }
        for (dot, conditional) in &prepaint.breakpoint_dots {
            let quad = if *conditional {
                fill(*dot, gpui::transparent_black()).border_widths(px(1.5)).border_color(theme.error)
            } else {
                fill(*dot, theme.error)
            };
            window.paint_quad(quad.corner_radii(px(4.)));
        }
        if let Some(band) = prepaint.assist_band {
            window.paint_quad(fill(band, theme.accent_soft));
            window.paint_quad(fill(gpui::Bounds::new(band.origin, gpui::size(px(3.), band.size.height)), theme.caret));
        }
        for (rect, color) in &prepaint.ai_tints {
            window.paint_quad(fill(*rect, *color));
        }
        for (rect, color) in &prepaint.git_marks {
            window.paint_quad(fill(*rect, *color).corner_radii(px(1.5)));
        }
        for (number, origin) in &prepaint.numbers {
            number.paint(*origin, line_height, window, cx).ok();
        }
        for (bounds, folded) in &prepaint.chevrons {
            // Folded points right, open points down.
            let angle = if *folded { 0. } else { std::f32::consts::FRAC_PI_2 };
            let center = bounds.center().scale(window.scale_factor());
            let turn = gpui::TransformationMatrix::unit()
                .translate(center)
                .rotate(gpui::radians(angle))
                .translate(gpui::Negate::negate(center));
            let color = if *folded { theme.muted } else { theme.faint };
            window.paint_svg(*bounds, "icons/chevron-right.svg".into(), turn, color, cx).ok();
        }
        window.with_content_mask(Some(ContentMask { bounds: prepaint.text_bounds }), |window| {
            for rect in &prepaint.indent_guides {
                window.paint_quad(fill(*rect, theme.hairline));
            }
            if let Some(rect) = prepaint.line_guide {
                window.paint_quad(fill(rect, theme.hairline));
            }
            for rect in &prepaint.symbol_marks {
                window.paint_quad(fill(*rect, theme.find_match.opacity(0.6)).corner_radii(px(3.)));
            }
            for (rect, current) in &prepaint.matches {
                let quad = fill(*rect, theme.find_match).corner_radii(px(3.));
                window.paint_quad(if *current { quad.border_widths(px(1.)).border_color(theme.caret) } else { quad });
            }
            for rect in &prepaint.selection {
                window.paint_quad(fill(*rect, theme.selection).corner_radii(px(3.)));
            }
            for (line, origin) in &prepaint.lines {
                line.paint(*origin, line_height, window, cx).ok();
            }
            for rect in &prepaint.bracket_boxes {
                window.paint_quad(
                    fill(*rect, gpui::transparent_black())
                        .border_widths(px(1.))
                        .border_color(theme.muted)
                        .corner_radii(px(2.)),
                );
            }
            for rect in &prepaint.link {
                window.paint_quad(fill(*rect, theme.foreground));
            }
            for rect in &prepaint.marked {
                window.paint_quad(fill(*rect, theme.foreground));
            }
            if let Some((rect, opacity)) = prepaint.caret
                && opacity > 0.
            {
                window.paint_quad(fill(rect, theme.caret.opacity(opacity)).corner_radii(px(1.)));
            }
            let opacity = prepaint.caret.map_or(1., |(_, o)| o);
            for rect in &prepaint.extra_carets {
                window.paint_quad(fill(*rect, theme.caret.opacity(opacity)).corner_radii(px(1.)));
            }
        });
        // The pinned first lines, over the code (and its gutter), with a line below them.
        if let Some((band, pinned)) = &prepaint.sticky {
            window.paint_quad(fill(*band, theme.background));
            window.with_content_mask(Some(ContentMask { bounds: prepaint.text_bounds }), |window| {
                for (text, origin, _, _) in pinned {
                    text.paint(*origin, line_height, window, cx).ok();
                }
            });
            for (_, _, number, at) in pinned {
                number.paint(*at, line_height, window, cx).ok();
            }
            let rule = Bounds::new(point(band.left(), band.bottom()), size(band.size.width, px(1.)));
            window.paint_quad(fill(rule, theme.hairline));
        }
        if let Some((thumb, emphasis)) = prepaint.scroll_thumb {
            let thumb = Bounds::new(
                point(thumb.left() + px(2.), thumb.top() + px(2.)),
                size(px(6.), thumb.size.height - px(4.)),
            );
            window.paint_quad(fill(thumb, theme.muted.opacity(emphasis)).corner_radii(px(3.)));
        }
        for (mark, color) in &prepaint.scroll_marks {
            window.paint_quad(fill(*mark, *color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_the_blocks_the_view_is_inside() {
        // impl 0..30 { fn 2..20 { for 4..15 { … } } fn 22..28 { … } }
        let foldable = [0..30, 2..20, 4..15, 22..28];
        let around = |line: usize| foldable.iter().filter(|r| r.start < line && line < r.end).cloned().collect();
        let from = |top: usize| move |n: usize| Some(top + n);
        // Inside the loop: impl, fn and for, and the pins cover the rows they hide.
        assert_eq!(sticky_lines(around, from(5), 3), [0, 2, 4]);
        // Just under the impl's first line: only the impl.
        assert_eq!(sticky_lines(around, from(1), 3), [0]);
        // At most two: the innermost two.
        assert_eq!(sticky_lines(around, from(5), 2), [2, 4]);
        // Near a block's end, once the pins would hide it, it lets go.
        assert_eq!(sticky_lines(around, from(13), 3), [0, 2]);
        assert_eq!(sticky_lines(around, from(20), 3), [0]);
        assert!(sticky_lines(around, from(0), 3).is_empty());
    }

    #[test]
    fn tabs_show_as_spaces_to_the_next_stop() {
        let font = gpui::font("Mono");
        let text = "a\tb\t\tc";
        let runs = [run(2, &font, gpui::black()), run(text.len() - 2, &font, gpui::white())];
        let (shown, runs, tabs) = expand_tabs(text, &runs);
        assert_eq!(shown, "a   b   ".to_string() + "    c");
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), shown.len());
        assert_eq!(runs[0].len, 4);
        let row = RowLayout {
            row: Row { line: 0, cols: 0..6, indent: 0, last: true, block: None },
            text: text.into(),
            shaped: ShapedLine::default(),
            x: px(0.),
            gaps: gaps_of(&[], &tabs),
        };
        // "b" is byte 2 of the text and byte 4 as shown, and back.
        assert_eq!(row.shown_byte(2), 4);
        assert_eq!(row.text_byte(4), 2);
        assert_eq!(row.text_byte(row.shown_byte(5)), 5);
        // A click inside a tab's spaces lands on its nearer edge.
        assert_eq!(row.text_byte(2), 1);
        assert_eq!(row.text_byte(3), 2);
    }

    #[test]
    fn hints_show_before_their_character_without_moving_the_text() {
        let font = gpui::font("Mono");
        let text = "let x = f(1);";
        let runs = [run(text.len(), &font, gpui::white())];
        // `x: i32` and `f(n: 1)`: hints at bytes 5 and 10.
        let hints = [(5, ": i32".to_string()), (10, "n: ".to_string())];
        let (shown, runs, placed) = insert_hints(text, &runs, &hints, |len| run(len, &font, gpui::black()));
        assert_eq!(shown, "let x: i32 = f(n: 1);");
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), shown.len());
        let row = RowLayout {
            row: Row { line: 0, cols: 0..13, indent: 0, last: true, block: None },
            text: text.into(),
            shaped: ShapedLine::default(),
            x: px(0.),
            gaps: gaps_of(&placed, &[]),
        };
        // The caret after `x` sits before its hint; ` =` comes after it.
        assert_eq!(row.shown_byte(5), 5);
        assert_eq!(row.shown_byte(6), 11);
        assert_eq!(row.shown_byte(10), 15);
        assert_eq!(row.shown_byte(11), 19);
        // A click on a hint lands before the character it stands for.
        assert_eq!(row.text_byte(8), 5);
        assert_eq!(row.text_byte(17), 10);
        assert_eq!(row.text_byte(row.shown_byte(12)), 12);
    }

    #[test]
    fn tabs_after_hints_keep_their_own_bytes() {
        // "a\tb" with a hint before the tab: the tab is still byte 1.
        let placed = [(1, 3)];
        let tabs = [(4, 2)];
        let gaps = gaps_of(&placed, &tabs);
        assert_eq!(gaps, vec![Gap { byte: 1, extra: 3, hint: true }, Gap { byte: 1, extra: 2, hint: false }]);
    }
}

#[cfg(test)]
mod long_lines {
    use super::*;
    use crate::buffer::Buffer;
    use gpui::TestAppContext;

    #[gpui::test]
    fn a_long_line_is_built_only_around_the_view(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            cx.set_global(Settings::default());
        });
        let line = "{\"id\":1},".repeat(10_000);
        let source = format!("{line}\nshort\n");
        let (editor, cx) = cx.add_window_view(|_, cx| {
            Editor::new(Buffer::from_text(&source), Some(std::path::PathBuf::from("big.json")), cx)
        });
        let size = gpui::size(px(1000.), px(600.));
        let draw = |cx: &mut gpui::VisualTestContext| {
            let editor = editor.clone();
            cx.draw(Point::default(), size, move |_, _| gpui::AnyView::from(editor));
        };
        // At the end of the line: the view scrolls there, and only that part is built.
        editor.update(cx, |e, cx| e.set_caret_point((0, usize::MAX), cx));
        draw(cx);
        draw(cx);
        editor.update(cx, |e, _| {
            // Too long to colour as you type: plain text.
            assert!(e.spans.is_empty());
            let layout = e.layout.as_ref().unwrap();
            let row = &layout.rows[0];
            assert!(row.text.len() < 2 * LONG_LINE_ROOM + 400, "built {} bytes", row.text.len());
            assert!(row.row.last && row.row.cols.end == line.len());
            // The short line below is whole.
            assert_eq!(layout.rows[1].text, "short");
        });
    }
}

#[cfg(test)]
mod timing {
    use super::*;
    use crate::buffer::Buffer;
    use gpui::TestAppContext;

    /// Not a check: how long a keystroke takes on a big file, from the edit to the editor
    /// drawn again (the test platform shapes text for free, so this is Null's own work).
    /// `cargo test --release timing -- --ignored --nocapture`
    #[gpui::test]
    #[ignore]
    fn keystroke_and_redraw_on_a_big_file(cx: &mut TestAppContext) {
        let source = std::fs::read_to_string("src/workspace.rs").unwrap();
        let none = Settings {
            inlay_hints: false,
            indent_guides: false,
            sticky_scroll: false,
            line_blame: false,
            symbol_marks: false,
            ..Settings::default()
        };
        let variants: Vec<(&str, Settings)> = vec![
            ("nothing extra", none.clone()),
            ("indent guides", Settings { indent_guides: true, ..none.clone() }),
            ("sticky scroll", Settings { sticky_scroll: true, ..none.clone() }),
            ("symbol marks", Settings { symbol_marks: true, ..none.clone() }),
            ("type hints", Settings { inlay_hints: true, ..none.clone() }),
            ("defaults", Settings::default()),
        ];
        cx.update(|cx| {
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            cx.set_global(none.clone());
        });
        let (editor, cx) = cx.add_window_view(|_, cx| {
            Editor::new(Buffer::from_text(&source), Some(std::path::PathBuf::from("big.rs")), cx)
        });
        let size = gpui::size(px(1400.), px(900.));
        let draw = |cx: &mut gpui::VisualTestContext| {
            let editor = editor.clone();
            cx.draw(Point::default(), size, move |_, _| gpui::AnyView::from(editor));
        };
        for (name, settings) in variants {
            cx.update(|_, cx| cx.set_global(settings));
            editor.update(cx, |e, cx| e.set_caret_point((3000, 8), cx));
            draw(cx);
            draw(cx);
            let (mut edit, mut redraw) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
            for _ in 0..20 {
                let t = std::time::Instant::now();
                editor.update(cx, |e, cx| {
                    let at = e.selection.head;
                    e.type_text_for_test(at, "x", cx);
                });
                edit += t.elapsed();
                let t = std::time::Instant::now();
                draw(cx);
                redraw += t.elapsed();
            }
            println!("{name:>14}: {:?} edit + {:?} redraw, per keystroke", edit / 20, redraw / 20);
        }
    }
}
