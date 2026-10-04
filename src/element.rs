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
const TOP_PADDING: f32 = 8.;
const TEXT_PADDING: f32 = 8.;
const GUTTER_PADDING: f32 = 16.;
/// Width of the scrollbar along the right edge.
const BAR: f32 = 10.;
/// Room in the gutter, after the line numbers, for the fold chevrons.
const FOLD_SPACE: f32 = 12.;

/// One row of text as drawn last frame.
pub struct RowLayout {
    pub row: Row,
    /// The row's text (part of a line when it wraps).
    pub text: String,
    pub shaped: ShapedLine,
    /// Where the text starts: past the indent on a continuation row.
    pub x: Pixels,
    /// Tabs, drawn as spaces: each one's byte in `text`, and the bytes it gained.
    pub tabs: Vec<(usize, usize)>,
}

impl RowLayout {
    /// Where a byte of `text` is in the text as shown.
    pub fn shown_byte(&self, byte: usize) -> usize {
        byte + self.tabs.iter().take_while(|(b, _)| *b < byte).map(|(_, g)| g).sum::<usize>()
    }

    /// The byte of `text` shown at `shown`; inside a tab's spaces, its nearer edge.
    pub fn text_byte(&self, shown: usize) -> usize {
        let mut gained = 0;
        for &(byte, extra) in &self.tabs {
            let start = byte + gained;
            if shown <= start {
                break;
            }
            if shown <= start + extra {
                return if shown - start <= extra / 2 { byte } else { byte + 1 };
            }
            gained += extra;
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
        Some(r) if r.row.line == line && r.row.block.is_none() => {
            let col = col.saturating_sub(r.row.cols.start);
            (row, r.x + r.shaped.x_for_index(r.shown_byte(byte_of_column(&r.text, col))))
        }
        _ => (row, char_width * display_col as f32),
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
    cuts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let syntax = colored.iter().find(|(r, _)| r.start <= a && b <= r.end).map_or(Syntax::Plain, |(_, s)| *s);
            let mut run = run(b - a, font, theme.syntax(syntax));
            if let Some((_, color)) = underlines.iter().find(|(r, _)| r.start <= a && b <= r.end) {
                run.underline = Some(UnderlineStyle { color: Some(*color), thickness: px(1.), wavy: true });
            }
            run
        })
        .collect()
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
                (shape(shown, &runs), tabs)
            };

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
            let wrap_width =
                cx.global::<Settings>().word_wrap.then(|| ((text_width - BAR - cw) / cw).floor().max(1.) as usize);
            editor.wrap.update(&editor.buffer, wrap_width, &editor.block_specs());
            let total_rows = editor.wrap.rows();
            let (caret_row, _) = editor.wrap.to_display(caret_line, caret_col, &editor.buffer);

            // Vertical scroll: follow the caret after keyboard moves, then glide toward the target.
            let max_y = total_rows.saturating_sub(1) as f32 * lh;
            // Dragging past an edge keeps scrolling, frame after frame.
            if editor.continue_drag(cx) {
                window.request_animation_frame();
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
            let rows: Vec<Row> = visible.clone().map(|r| editor.wrap.row(r, &editor.buffer)).collect();
            let lines_shown = match (rows.first(), rows.last()) {
                (Some(a), Some(b)) => a.line..b.line + 1,
                _ => 0..0,
            };
            let texts: Vec<String> = lines_shown.clone().map(|l| editor.buffer.line_text(l)).collect();
            editor.highlight_lines(lines_shown.clone());
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
                                return RowLayout { x: px(0.), row, text, shaped, tabs };
                            }
                            crate::editor::BlockKind::Ghost(lines) => {
                                // The rest of a ghost completion: faint, nothing struck.
                                let text = lines.get(i).cloned().unwrap_or_default();
                                let (shaped, tabs) = shape_row(&text, &[run(text.len(), &font, theme.faint)]);
                                return RowLayout { x: px(0.), row, text, shaped, tabs };
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
                        return RowLayout { x: px(0.), row, text, shaped, tabs };
                    }
                    let i = row.line - lines_shown.start;
                    let line_text = &texts[i];
                    let (b0, b1) = (byte_of_column(line_text, row.cols.start), byte_of_column(line_text, row.cols.end));
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
                        return RowLayout { x: char_width * row.indent as f32, row, text, shaped, tabs };
                    }
                    // The code the AI is rewriting fades while the new code appears.
                    if editor.ai_writing_lines().is_some_and(|l| l.contains(&row.line)) {
                        let (shaped, tabs) = shape_row(&text, &[run(text.len(), &font, theme.faint)]);
                        return RowLayout { x: char_width * row.indent as f32, row, text, shaped, tabs };
                    }
                    let mut runs = runs_for(&text, line_byte, &editor.spans, &row_underlines, &theme, &font);
                    // Who last changed the caret's line, faintly after its end.
                    if row.last
                        && show_blame
                        && !editor.is_folded(row.line)
                        && let Some((line, blame)) = editor.line_blame()
                        && line == row.line
                    {
                        let note = format!("{BLAME_GAP}{blame}");
                        runs.push(run(note.len(), &font, theme.faint));
                        let (shaped, tabs) = shape_row(&format!("{text}{note}"), &runs);
                        return RowLayout { x: char_width * row.indent as f32, row, text, shaped, tabs };
                    }
                    // A folded line ends in "⋯", standing in for the lines it hides.
                    if row.last && editor.is_folded(row.line) {
                        runs.push(run(FOLDED.len(), &font, theme.muted));
                        let (shaped, tabs) = shape_row(&format!("{text}{FOLDED}"), &runs);
                        return RowLayout { x: char_width * row.indent as f32, row, text, shaped, tabs };
                    }
                    let (shaped, tabs) = shape_row(&text, &runs);
                    RowLayout { x: char_width * row.indent as f32, row, text, shaped, tabs }
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
                .map(|lines| rows_of(lines))
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
            let current_line = editor.selection.is_empty().then(|| {
                Bounds::new(
                    point(bounds.left(), row_top(caret_rows.start)),
                    size(bounds.size.width, line_height * caret_rows.len() as f32),
                )
            });

            // Fold chevrons: on folded lines always, on lines that can fold while the mouse
            // is over the gutter. Quiet otherwise.
            let chevrons = row_layouts
                .iter()
                .zip(visible.clone())
                .filter(|(r, _)| r.row.cols.start == 0 && r.row.block.is_none())
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
                .filter(|(r, _)| r.row.cols.start == 0 && r.row.block.is_none())
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
            let symbol_marks: Vec<Bounds<Pixels>> = {
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

            editor.layout = Some(Layout {
                text_origin: origin,
                text_bounds,
                bounds,
                line_height,
                char_width,
                first_row: visible.start,
                rows: row_layouts,
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
            tabs,
        };
        // "b" is byte 2 of the text and byte 4 as shown, and back.
        assert_eq!(row.shown_byte(2), 4);
        assert_eq!(row.text_byte(4), 2);
        assert_eq!(row.text_byte(row.shown_byte(5)), 5);
        // A click inside a tab's spaces lands on its nearer edge.
        assert_eq!(row.text_byte(2), 1);
        assert_eq!(row.text_byte(3), 2);
    }
}
