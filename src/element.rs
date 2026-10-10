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
/// shown, runs to match (each hint in `hint_run(its index, its length)`'s style), and each
/// hint's (byte, length).
fn insert_hints(
    text: &str,
    runs: &[TextRun],
    hints: &[(usize, String)],
    hint_run: impl Fn(usize, usize) -> TextRun,
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
            out.push(hint_run(placed.len(), hint.len()));
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
        out.push(hint_run(placed.len(), hint.len()));
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

/// Between the parts of a code lens.
const LENS_SEP: &str = "  ·  ";
/// Shown before a color written in the code, in that color.
const SWATCH: &str = "■ ";
/// Shown instead for a color that would hardly show on the background.
const OUTLINE_SWATCH: &str = "□ ";
/// A problem's message at the end of its line is cut to this many characters.
const PROBLEM_NOTE_CHARS: usize = 100;

/// The spaces and tabs of a row's `text` (starting at char `start` of the file) to mark:
/// those inside one of the `selected` ranges, and as `shown` says, those at the end of the
/// line (when the row ends it) or all of them. Their columns in the row, and whether each is
/// a tab.
fn whitespace_marks(
    text: &str,
    start: usize,
    selected: &[Range<usize>],
    shown: crate::settings::ShowWhitespace,
    ends_line: bool,
) -> Vec<(usize, bool)> {
    use crate::settings::ShowWhitespace;
    let blank = |c: char| c == ' ' || c == '\t';
    // Where the line's own spaces at its end start (a line of only spaces: from its start).
    let trailing_from = match shown {
        ShowWhitespace::Trailing if ends_line => {
            text.chars().enumerate().filter(|(_, c)| !blank(*c)).last().map_or(0, |(last, _)| last + 1)
        }
        _ => usize::MAX,
    };
    text.chars()
        .enumerate()
        .filter(|(_, c)| blank(*c))
        .filter(|(i, _)| {
            shown == ShowWhitespace::All || *i >= trailing_from || selected.iter().any(|r| r.contains(&(start + i)))
        })
        .map(|(i, c)| (i, c == '\t'))
        .collect()
}

/// Where the character at column `col` of a row's text is drawn, its glyphs looked for in
/// any order (right-to-left text is drawn in another order than it's written).
fn glyph_x(r: &RowLayout, col: usize) -> Pixels {
    let Some((byte, _)) = r.text.char_indices().nth(col) else { return r.shaped.width };
    let before: usize = r.gaps.iter().filter(|g| g.byte < byte).map(|g| g.extra).sum();
    let hint: usize = r.gaps.iter().filter(|g| g.byte == byte && g.hint).map(|g| g.extra).sum();
    let index = byte + before + hint;
    let glyphs = || r.shaped.runs.iter().flat_map(|run| run.glyphs.iter());
    // Its own glyph; else (a character drawn as nothing) the one after it in the text.
    glyphs()
        .filter(|g| g.index == index)
        .map(|g| g.position.x)
        .reduce(|a, b| if b < a { b } else { a })
        .or_else(|| glyphs().filter(|g| g.index > index).min_by_key(|g| g.index).map(|g| g.position.x))
        .unwrap_or(r.shaped.width)
}

/// Where each of `marks` (columns of the row's text, and whether a tab) starts and ends in
/// the row as shown (after a hint shown before it), in one walk along the row's characters
/// and one along its glyphs: per frame, on every visible row.
fn whitespace_xs(r: &RowLayout, marks: &[(usize, bool)]) -> Vec<(Pixels, Pixels, bool)> {
    let mut indices = Vec::with_capacity(marks.len() * 2);
    let mut chars = r.text.char_indices().enumerate();
    let mut gaps = r.gaps.iter().peekable();
    let mut before = 0; // extra shown for gaps before the byte reached
    for &(col, _) in marks {
        let Some((_, (byte, c))) = chars.by_ref().find(|(i, _)| *i == col) else { break };
        while gaps.peek().is_some_and(|g| g.byte < byte) {
            before += gaps.next().map_or(0, |g| g.extra);
        }
        // A hint shown before this character: the mark goes after it.
        let hint: usize = r.gaps.iter().filter(|g| g.byte == byte && g.hint).map(|g| g.extra).sum();
        let at: usize = r.gaps.iter().filter(|g| g.byte == byte).map(|g| g.extra).sum();
        indices.push(byte + before + hint);
        indices.push(byte + c.len_utf8() + before + at);
    }
    // The x of each index: the first glyph at or past it.
    let mut glyphs = r.shaped.runs.iter().flat_map(|run| run.glyphs.iter()).peekable();
    let xs: Vec<Pixels> = indices
        .iter()
        .map(|&i| {
            while glyphs.peek().is_some_and(|g| g.index < i) {
                glyphs.next();
            }
            glyphs.peek().map_or(r.shaped.width, |g| g.position.x)
        })
        .collect();
    marks.iter().zip(xs.chunks(2)).map(|(&(_, tab), x)| (x[0], x[1], tab)).collect()
}

/// `note` in at most `room` characters: cut short with "…". None when even its start
/// wouldn't fit (a few letters say nothing).
fn shortened(note: &str, room: usize) -> Option<String> {
    if note.chars().count() <= room {
        return Some(note.to_string());
    }
    if room < BLAME_GAP.chars().count() + 8 {
        return None;
    }
    let mut cut: String = note.chars().take(room - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    Some(cut)
}

/// The note at the end of the caret's line when something's wrong there: the most serious
/// problem starting on it (`chars` of the text), its first line, cut short.
fn problem_note<'a>(
    problems: impl IntoIterator<Item = &'a crate::editor::Problem>,
    chars: Range<usize>,
) -> Option<(String, DiagnosticSeverity)> {
    let rank = |s: DiagnosticSeverity| match s {
        DiagnosticSeverity::ERROR => 0,
        DiagnosticSeverity::WARNING => 1,
        DiagnosticSeverity::INFORMATION => 2,
        _ => 3,
    };
    let problem = problems
        .into_iter()
        .filter(|p| chars.contains(&p.range.start) && rank(p.severity) <= 1)
        .min_by_key(|p| rank(p.severity))?;
    let first = problem.message.lines().next().unwrap_or("").trim();
    let note = match first.char_indices().nth(PROBLEM_NOTE_CHARS) {
        Some((cut, _)) => format!("{}…", &first[..cut]),
        None => first.to_string(),
    };
    Some((note, problem.severity))
}

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
    /// Each breakpoint's dot, and what kind: 0 plain, 1 with a condition or count, 2 a
    /// log point (prints, doesn't stop).
    breakpoint_dots: Vec<(Bounds<Pixels>, u8)>,
    /// Bookmarked lines when line numbers are hidden (shown, the number takes the accent).
    bookmark_dots: Vec<Bounds<Pixels>>,
    /// The line the debugger stopped on: a band across it, and a mark in the gutter.
    execution: Option<(Bounds<Pixels>, Bounds<Pixels>)>,
    /// Faint lines down the indentation, one per level.
    /// Each guide, and whether it's the one of the block the caret is in (a little brighter).
    indent_guides: Vec<(Bounds<Pixels>, bool)>,
    /// The line at the project's line length.
    line_guide: Option<Bounds<Pixels>>,
    /// The first lines of the blocks the view is inside, pinned at the top: the band
    /// behind them, and each one's text and line number.
    #[allow(clippy::type_complexity)]
    sticky: Option<(Bounds<Pixels>, Vec<(ShapedLine, Point<Pixels>, ShapedLine, Point<Pixels>)>)>,
    numbers: Vec<(ShapedLine, Point<Pixels>)>,
    /// Fold chevrons: where, and whether folded (pointing right) or open (down).
    chevrons: Vec<(Bounds<Pixels>, bool)>,
    lines: Vec<(ShapedLine, Point<Pixels>)>,
    selection: Vec<Bounds<Pixels>>,
    /// Spaces (a dot) and tabs (a dash) inside the selection, so what's selected shows.
    whitespace: Vec<Bounds<Pixels>>,
    /// Characters that can't be seen as themselves, each in a dashed box (narrow for one
    /// with no width), red for one that turns text around.
    invisibles: Vec<(Bounds<Pixels>, crate::editor::invisible::Invisible)>,
    matches: Vec<(Bounds<Pixels>, bool)>,
    /// Other uses of the symbol at the caret.
    symbol_marks: Vec<Bounds<Pixels>>,
    link: Vec<Bounds<Pixels>>,
    git_marks: Vec<(Bounds<Pixels>, Hsla)>,
    assist_band: Option<Bounds<Pixels>>,
    ai_tints: Vec<(Bounds<Pixels>, Hsla)>,
    /// Over the rows outside the paragraph being written, when the others fade.
    veils: Vec<Bounds<Pixels>>,
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
/// How a stretch of a line is marked.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mark {
    /// A wavy underline: a problem, a broken link, a misspelling.
    Wavy(Hsla),
    /// Faded: code the language server says isn't used (or is switched off).
    Faded,
    /// Struck through: a deprecated name.
    Struck,
}

fn runs_for(
    text: &str,
    line_start: usize,
    spans: &[Span],
    underlines: &[(Range<usize>, Mark)],
    theme: &Theme,
    font: &Font,
) -> Vec<TextRun> {
    let colored: Vec<Span> = spans_in(spans, line_start..line_start + text.len()).collect();
    // Cut the line wherever a color or a mark starts or ends.
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
            let marks = underlines.iter().filter(|(r, _)| r.start <= a && b <= r.end).map(|(_, mark)| *mark);
            let mut wavy = None;
            let mut faded = false;
            for mark in marks {
                match mark {
                    Mark::Wavy(color) => wavy = wavy.or(Some(color)),
                    // Once, however many problems say so.
                    Mark::Faded => faded = true,
                    Mark::Struck => {}
                }
            }
            if faded {
                run.color = run.color.opacity(0.45);
            }
            if let Some(color) = wavy {
                run.underline = Some(UnderlineStyle { color: Some(color), thickness: px(1.), wavy: true });
            }
            if underlines.iter().any(|(r, mark)| r.start <= a && b <= r.end && *mark == Mark::Struck) {
                run.strikethrough = Some(StrikethroughStyle { thickness: px(1.), color: Some(run.color) });
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
            font.features = crate::fonts::code_features(editor.ligatures(cx));
            let text_system = window.text_system().clone();
            let shape = |text: String, runs: &[TextRun]| text_system.shape_line(text.into(), font_size, runs, None);
            // A row of text, with its tabs drawn as spaces.
            let shape_row = |text: &str, runs: &[TextRun]| {
                let (shown, runs, tabs) = expand_tabs(text, runs);
                (shape(shown, &runs), gaps_of(&[], &tabs))
            };
            // Type hints: faint, on a soft background, in the code's own font. A color's
            // swatch: a square in that color.
            let hint_style = |len: usize, swatch: Option<Hsla>| match swatch {
                Some(color) => run(len, &font, color),
                None => TextRun { background_color: Some(theme.hairline), ..run(len, &font, theme.faint) },
            };
            // A row with its hints in (swatches among them, by index), then `suffix` (a
            // fold's ⋯, who changed the line), in its runs.
            let shape_hinted = |text: &str,
                                runs: &[TextRun],
                                hints: &[(usize, String)],
                                swatches: &[Option<Hsla>],
                                suffix: Vec<TextRun>,
                                suffix_text: &str| {
                let (mut shown, mut runs, placed) =
                    insert_hints(text, runs, hints, |i, len| hint_style(len, swatches.get(i).copied().flatten()));
                shown.push_str(suffix_text);
                runs.extend(suffix);
                let (shown, runs, tabs) = expand_tabs(&shown, &runs);
                (shape(shown, &runs), gaps_of(&placed, &tabs))
            };
            let show_hints = cx.global::<Settings>().inlay_hints;
            let shows_colors = editor.language().is_some_and(|l| crate::colors::written_in(l.name));
            if show_hints {
                editor.ensure_hints(cx);
            }
            // The caret's line: what the server says over it ("3 references", "▶ Run Test").
            let lens: Vec<crate::editor::lens::LensItem> = if cx.global::<Settings>().code_lens {
                editor.ensure_lenses(cx);
                let line = editor.buffer.point(editor.shown_caret(cx)).0;
                editor.lens_on_line(line, cx)
            } else {
                Vec::new()
            };
            let lens_note = (!lens.is_empty())
                .then(|| format!("{BLAME_GAP}{}", lens.iter().map(|l| l.title.as_str()).collect::<Vec<_>>().join(LENS_SEP)));

            let char_width = shape("0".repeat(10), &[run(10, &font, theme.foreground)]).width / 10.;
            let total_lines = editor.buffer.len_lines();
            // Without line numbers the gutter keeps only its marks: changes, folds, breakpoints.
            let numbered = editor.shows_line_numbers(cx);
            let digits = if numbered { total_lines.to_string().len().max(3) } else { 0 };
            let gutter_width = char_width * digits as f32 + px(GUTTER_PADDING * 2. + FOLD_SPACE);
            let text_bounds =
                Bounds::from_corners(point(bounds.left() + gutter_width, bounds.top()), bounds.bottom_right());
            let viewport_height = f32::from(bounds.size.height);
            // (Vim's Visual mode: on the last character selected, not after it.)
            let (caret_line, caret_col) = editor.buffer.point(editor.shown_caret(cx));
            let text_width = f32::from(text_bounds.size.width) - TEXT_PADDING;
            let cw = f32::from(char_width);

            // Word wrap: rows as wide as the text area (clear of the scrollbar), in characters;
            // or, asked for, as the project's line length when that's narrower.
            let guide = editor.style.ruler.filter(|_| cx.global::<Settings>().wrap_at_guide);
            let wrap_width = editor.wraps(cx).then(|| {
                let window = ((text_width - BAR - cw) / cw).floor().max(1.) as usize;
                guide.map_or(window, |guide| window.min(guide.max(1)))
            });
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
            let typewriter = cx.global::<Settings>().typewriter;
            if editor.autoscroll && (editor.center_once || (typewriter && !editor.reveal_only)) {
                // Typewriter (or ⌃L): the caret's line in the middle, from the keyboard (a click
                // doesn't pull the text about).
                let y = caret_row as f32 * lh + TOP_PADDING;
                editor.scroll.target_y = (y + lh / 2. - viewport_height / 2.).max(0.);
            } else if editor.autoscroll {
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
            if let Some(bytes) = &shown_bytes {
                editor.highlight_bytes(bytes.clone());
            }
            editor.ensure_meaning(cx);
            // The server's colours over the grammar's, when it has said what names are.
            let meant = editor.spans_to_draw(shown_bytes.unwrap_or_default(), cx.global::<Settings>().bracket_colours);
            // The lines that can fold, needed only while the mouse is over the gutter.
            let over_gutter = editor.mouse_position.is_some_and(|p| {
                p.x >= bounds.left() && p.x < text_bounds.left() && p.y >= bounds.top() && p.y < bounds.bottom()
            });
            let foldable: std::collections::HashSet<usize> =
                if over_gutter { editor.foldable().iter().map(|r| r.start).collect() } else { Default::default() };

            // Errors and warnings get a wavy underline and color their line number.
            // Hints and notes only show in the hover card, to keep the code calm; code the
            // server says is unused (or switched off) fades, a deprecated name is struck through.
            let mut underlines: Vec<Vec<(Range<usize>, Mark)>> = vec![Vec::new(); lines_shown.len()];
            let mut flagged: Vec<Option<Hsla>> = vec![None; lines_shown.len()];
            for problem in editor.problems(cx).iter() {
                let color = match problem.severity {
                    DiagnosticSeverity::ERROR => Some(theme.error),
                    DiagnosticSeverity::WARNING => Some(theme.warning),
                    _ => None,
                };
                let tagged = |tag| problem.diagnostic.tags.as_ref().is_some_and(|t| t.contains(&tag));
                let faded = tagged(lsp_types::DiagnosticTag::UNNECESSARY);
                let struck = tagged(lsp_types::DiagnosticTag::DEPRECATED);
                if color.is_none() && !faded && !struck {
                    continue;
                }
                let ((start_line, start_col), (end_line, end_col)) = (problem.start, problem.end);
                if end_line < lines_shown.start || start_line >= lines_shown.end {
                    continue;
                }
                for line in start_line.max(lines_shown.start)..=end_line.min(lines_shown.end.saturating_sub(1)) {
                    let i = line - lines_shown.start;
                    let text = &texts[i];
                    let from = if line == start_line { start_col } else { 0 };
                    let to = if line == end_line { end_col } else { text.chars().count() };
                    let range = byte_of_column(text, from)..byte_of_column(text, to);
                    if to > from {
                        for (on, mark) in [(faded, Mark::Faded), (struck, Mark::Struck)] {
                            if on {
                                underlines[i].push((range.clone(), mark));
                            }
                        }
                    }
                    let Some(color) = color else { continue };
                    // Zero-width problems still get one character underlined.
                    let wavy = if to > from { range } else { range.start..byte_of_column(text, from + 1) };
                    underlines[i].push((wavy, Mark::Wavy(color)));
                    if line == start_line && flagged[i] != Some(theme.error) {
                        flagged[i] = Some(color);
                    }
                }
            }
            // Links to nothing (Markdown), then misspelled words faintly: after the problems,
            // so a problem's colour wins.
            for (i, line) in lines_shown.clone().enumerate() {
                for link in editor.broken_links_on_line(line, &texts[i]) {
                    underlines[i].push((link, Mark::Wavy(theme.warning)));
                }
                for word in editor.misspellings_on_line(line, &texts[i], cx) {
                    underlines[i].push((word, Mark::Wavy(theme.muted)));
                }
            }

            let show_blame = cx.global::<crate::settings::Settings>().line_blame;
            // Something wrong on the caret's line: said at its end, before who changed it; with
            // "problems at line ends" on, on every line, fainter away from the caret.
            let every_line = cx.global::<crate::settings::Settings>().problems_at_line_ends;
            let line_notes: std::collections::HashMap<usize, (String, Hsla)> = {
                let rope = editor.buffer.rope();
                let noted = if every_line { lines_shown.clone() } else { caret_line..caret_line + 1 };
                // Only those starting on the lines noted, looked through once.
                let all = editor.problems(cx);
                let problems: Vec<&crate::editor::Problem> =
                    all.iter().filter(|p| noted.contains(&p.start.0)).collect();
                noted
                    .filter_map(|line| {
                        // Problems are in chars.
                        let chars = rope.line_to_char(line.min(rope.len_lines()))
                            ..rope.line_to_char((line + 1).min(rope.len_lines()));
                        let chars = chars.start..chars.end.max(chars.start + 1);
                        problem_note(problems.iter().copied(), chars).map(|(note, severity)| {
                            let color = if severity == DiagnosticSeverity::ERROR { theme.error } else { theme.warning };
                            let strength = if line == caret_line { 0.75 } else { 0.5 };
                            (line, (format!("{BLAME_GAP}{note}"), color.opacity(strength)))
                        })
                    })
                    .collect()
            };
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
                    let row_underlines: Vec<(Range<usize>, Mark)> = underlines[i]
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
                        let split = |r: &(Range<usize>, Mark), from: usize, to: usize| {
                            let (start, end) = (r.0.start.max(from), r.0.end.min(to));
                            (start < end).then(|| (start - from..end - from, r.1))
                        };
                        let under_before: Vec<_> = row_underlines.iter().filter_map(|r| split(r, 0, at)).collect();
                        let under_after: Vec<_> =
                            row_underlines.iter().filter_map(|r| split(r, at, text.len())).collect();
                        let mut runs = runs_for(
                            before,
                            line_byte,
                            meant.as_deref().unwrap_or(&editor.spans),
                            &under_before,
                            &theme,
                            &font,
                        );
                        runs.push(run(ghost.len(), &font, theme.faint));
                        runs.extend(runs_for(
                            after,
                            line_byte + at,
                            meant.as_deref().unwrap_or(&editor.spans),
                            &under_after,
                            &theme,
                            &font,
                        ));
                        let shown = format!("{before}{ghost}{after}");
                        let (shaped, tabs) = shape_row(&shown, &runs);
                        return RowLayout { x: char_width * row.indent as f32, row, text, shaped, gaps: tabs };
                    }
                    // The code the AI is rewriting fades while the new code appears.
                    if editor.ai_writing_lines().is_some_and(|l| l.contains(&row.line)) {
                        let (shaped, tabs) = shape_row(&text, &[run(text.len(), &font, theme.faint)]);
                        return RowLayout { x: char_width * row.indent as f32, row, text, shaped, gaps: tabs };
                    }
                    let runs = runs_for(
                        &text,
                        line_byte,
                        meant.as_deref().unwrap_or(&editor.spans),
                        &row_underlines,
                        &theme,
                        &font,
                    );
                    // The row's type hints, at their bytes in its text.
                    let mut hints: Vec<(usize, String, Option<Hsla>)> = if show_hints {
                        editor
                            .hints_on_line(row.line)
                            .filter(|(col, _)| {
                                (row.cols.start..row.cols.end).contains(col) || (row.last && *col == row.cols.end)
                            })
                            .map(|(col, label)| (byte_of_column(&text, col - row.cols.start), label.to_string(), None))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    // Colors written in the code: a square of each, just before it.
                    if shows_colors {
                        let swatches = crate::colors::colors_in(&text);
                        hints.extend(swatches.into_iter().map(|(byte, color)| {
                            // One that would hardly show on the background: an outline instead.
                            if crate::colors::blends_into(color, theme.background) {
                                (byte, OUTLINE_SWATCH.to_string(), Some(theme.muted))
                            } else {
                                (byte, SWATCH.to_string(), Some(color))
                            }
                        }));
                        hints.sort_by_key(|(byte, _, swatch)| (*byte, swatch.is_none()));
                    }
                    let (hints, swatches): (Vec<(usize, String)>, Vec<Option<Hsla>>) =
                        hints.into_iter().map(|(byte, text, swatch)| ((byte, text), swatch)).unzip();
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
                    let lens_here = lens_note.clone().filter(|_| row.last && row.line == caret_line);
                    let (suffix, suffix_text) = if row.last && editor.is_folded(row.line) {
                        (vec![run(FOLDED.len(), &font, theme.muted)], FOLDED.to_string())
                    } else if let Some(note) = values {
                        (vec![run(note.len(), &font, theme.muted)], note)
                    } else if let Some(note) = tip {
                        (vec![run(note.len(), &font, theme.faint)], note)
                    } else if let Some((note, color)) =
                        line_notes.get(&row.line).filter(|_| row.last && !editor.is_folded(row.line))
                    {
                        // The problem, then what the server offers on the line (after it, to
                        // be clicked: see `lens_hits`). With lines wrapped nothing scrolls
                        // sideways: a problem too long for what's left of the row is cut
                        // short, or left to the squiggle and the status bar, so the lens
                        // stays in view.
                        let note = match &lens_here {
                            Some(lens) if editor.wrap.is_on() => {
                                let used = row.indent + crate::wrap::columns_of(&text);
                                let room = (text_width / cw) as usize;
                                let left = room.saturating_sub(used + lens.chars().count() + 1);
                                shortened(note, left).unwrap_or_default()
                            }
                            _ => note.clone(),
                        };
                        let mut runs = if note.is_empty() { Vec::new() } else { vec![run(note.len(), &font, *color)] };
                        let mut text = note;
                        if let Some(lens) = lens_here {
                            runs.push(run(lens.len(), &font, theme.muted));
                            text.push_str(&lens);
                        }
                        (runs, text)
                    } else if let Some(note) = lens_here {
                        (vec![run(note.len(), &font, theme.muted)], note)
                    } else if let Some(note) = blame {
                        (vec![run(note.len(), &font, theme.faint)], note)
                    } else {
                        (Vec::new(), String::new())
                    };
                    let (shaped, gaps) = shape_hinted(&text, &runs, &hints, &swatches, suffix, &suffix_text);
                    RowLayout { x: char_width * row.indent as f32, row, text, shaped, gaps }
                })
                .collect();
            // A tag's name with the caret in it, and its pair's (asked for before the rows
            // are borrowed below).
            let tag_pair = editor.tag_pair_at_caret();
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
            editor.center_once = false;
            editor.reveal_only = false;

            let origin = point(
                text_bounds.left() + px(TEXT_PADDING - editor.scroll.x),
                bounds.top() + px(TOP_PADDING - editor.scroll.y),
            );
            let row_top = |row: usize| origin.y + line_height * row as f32;
            // Where each part of the caret's line's lens is drawn: a click there does it.
            let mut lens_hits = Vec::new();
            if let Some(note) = &lens_note
                && let Some((i, r)) = row_layouts.iter().enumerate().find(|(_, r)| {
                    r.row.last && r.row.block.is_none() && r.row.line == caret_line && r.shaped.text.ends_with(note.as_str())
                })
            {
                let mut at = r.shaped.text.len() - note.len() + BLAME_GAP.len();
                let top = row_top(visible.start + i);
                for item in &lens {
                    let (x0, x1) = (r.shaped.x_for_index(at), r.shaped.x_for_index(at + item.title.len()));
                    lens_hits.push((
                        Bounds::from_corners(
                            point(origin.x + r.x + x0, top),
                            point(origin.x + r.x + x1, top + line_height),
                        ),
                        item.action.clone(),
                    ));
                    at += item.title.len() + LENS_SEP.len();
                }
            }
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
            // Writing with the other paragraphs faded: a veil over each row outside this one.
            let veils: Vec<Bounds<Pixels>> = match editor.focused_paragraph(cx) {
                Some(paragraph) => row_layouts
                    .iter()
                    .zip(visible.clone())
                    .filter(|(r, _)| !paragraph.contains(&r.row.line))
                    .map(|(_, row)| {
                        Bounds::new(point(text_bounds.left(), row_top(row)), size(text_bounds.size.width, line_height))
                    })
                    .collect(),
                None => Vec::new(),
            };
            // In a changed line, the words that changed: a little stronger, on each side.
            let words = editor.word_changes(lines_shown.clone());
            for (r, row) in row_layouts.iter().zip(visible.clone()) {
                let (ranges, color, cols) = match r.row.block {
                    Some(block) => {
                        (words.removed.get(&block), theme.git_deleted.opacity(0.3), 0..r.text.chars().count())
                    }
                    None => (words.added.get(&r.row.line), theme.git_added.opacity(0.3), r.row.cols.clone()),
                };
                for range in ranges.into_iter().flatten() {
                    let (from, to) = (range.start.max(cols.start), range.end.min(cols.end));
                    if from >= to {
                        continue;
                    }
                    let byte =
                        |col: usize| r.text.char_indices().nth(col - cols.start).map_or(r.text.len(), |(b, _)| b);
                    let x = |col: usize| origin.x + r.x + r.shaped.x_for_index(r.shown_byte(byte(col)));
                    let rect =
                        Bounds::from_corners(point(x(from), row_top(row)), point(x(to), row_top(row) + line_height));
                    ai_tints.push((rect, color));
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
            let breakpoint_dots: Vec<(Bounds<Pixels>, u8)> = editor
                .breakpoints
                .iter()
                .filter(|&&line| lines_shown.contains(&line))
                .filter_map(|&line| {
                    let rows = rows_of(line..line + 1);
                    (!rows.is_empty()).then(|| {
                        let y = row_top(rows.start) + (line_height - dot) / 2.;
                        let kind = match editor
                            .breakpoint_conditions
                            .iter()
                            .find(|(l, _)| *l == line)
                            .map(|(_, text)| crate::editor::BreakWhen::read(text))
                        {
                            Some(crate::editor::BreakWhen::Log(_)) => 2,
                            Some(crate::editor::BreakWhen::Condition("")) | None => 0,
                            Some(_) => 1,
                        };
                        (Bounds::new(point(bounds.left() + px(5.), y), size(dot, dot)), kind)
                    })
                })
                .collect();
            let mark = px(6.);
            let bookmark_dots: Vec<Bounds<Pixels>> = editor
                .bookmarks
                .iter()
                .filter(|_| !numbered)
                .filter(|&&line| lines_shown.contains(&line))
                .filter_map(|&line| {
                    let rows = rows_of(line..line + 1);
                    (!rows.is_empty()).then(|| {
                        // Inside a breakpoint's dot, smaller, so both show.
                        let mark = if editor.breakpoints.contains(&line) { px(4.) } else { mark };
                        let y = row_top(rows.start) + (line_height - mark) / 2.;
                        Bounds::new(point(bounds.left() + px(9.) - mark / 2., y), size(mark, mark))
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
            let indent_guides: Vec<(Bounds<Pixels>, bool)> = if cx.global::<Settings>().indent_guides {
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
                // The caret's block: the guide just left of its line's text, over the lines
                // around it at least that far in.
                let active = caret_line.checked_sub(lines_shown.start).and_then(|at| {
                    let level = *levels.get(at)?;
                    let guide = level.checked_sub(1)?;
                    let inside = |i: usize| levels.get(i).is_some_and(|l| *l > guide);
                    let first = (0..=at).rev().take_while(|&i| inside(i)).last()?;
                    let last = (at..levels.len()).take_while(|&i| inside(i)).last()?;
                    Some((guide, lines_shown.start + first..lines_shown.start + last + 1))
                });
                row_layouts
                    .iter()
                    .zip(visible.clone())
                    .filter(|(r, _)| r.row.block.is_none())
                    .flat_map(|(r, row)| {
                        let line = r.row.line;
                        let level = levels.get(line - lines_shown.start).copied().unwrap_or(0);
                        let active = active.clone();
                        (0..level).map(move |k| {
                            let x = origin.x + char_width * (k * unit) as f32;
                            let lit =
                                active.as_ref().is_some_and(|(guide, lines)| *guide == k && lines.contains(&line));
                            (Bounds::new(point(x.round(), row_top(row)), size(px(1.), line_height)), lit)
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
                .filter(|_| numbered)
                .filter(|(r, row)| editor.wrap.first_row(r.row.line) == *row && r.row.block.is_none())
                .map(|(r, row)| {
                    let line = r.row.line;
                    let label = (line + 1).to_string();
                    // A bookmark shows over a problem's colour: the problem has its wavy line.
                    let bookmarked = editor.is_bookmarked(line).then_some(theme.caret);
                    let color = bookmarked.or(flagged[line - lines_shown.start]).unwrap_or(if line == caret_line {
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
            // In the selection, on the rows shown: a dot for each space, a dash for each tab.
            let mut whitespace = Vec::new();
            let selected: Vec<Range<usize>> = std::iter::once(selection_range.clone())
                .chain(editor.extra.iter().map(|c| c.selection.range()))
                .filter(|r| !r.is_empty())
                .collect();
            let shown_whitespace = cx.global::<Settings>().whitespace;
            if !selected.is_empty() || shown_whitespace != crate::settings::ShowWhitespace::Selection {
                for (i, r) in row_layouts.iter().enumerate() {
                    if r.row.block.is_some() {
                        continue;
                    }
                    let row_start = editor.buffer.line_to_char(r.row.line) + r.row.cols.start;
                    let top = row_top(visible.start + i);
                    // A wrapped row ends the line's text when all that's after it is blank.
                    // (Read from the rope after the row, not from a copy of the whole line.)
                    let ends_line = r.row.last
                        || (shown_whitespace == crate::settings::ShowWhitespace::Trailing && {
                            let rope = editor.buffer.rope();
                            let from = rope.line_to_char(r.row.line) + r.row.cols.end;
                            let to = rope.line_to_char(r.row.line) + editor.buffer.line_len(r.row.line);
                            rope.slice(from.min(to)..to).chars().all(|c| c == ' ' || c == '\t')
                        });
                    let marks = whitespace_marks(&r.text, row_start, &selected, shown_whitespace, ends_line);
                    if marks.is_empty() {
                        continue;
                    }
                    for (x0, x1, tab) in whitespace_xs(r, &marks) {
                        let mid = origin.x + r.x + (x0 + x1) / 2.;
                        let y = top + line_height / 2.;
                        whitespace.push(if tab {
                            let half = (x1 - x0) * 0.35;
                            Bounds::from_corners(point(mid - half, y - px(0.5)), point(mid + half, y + px(0.5)))
                        } else {
                            Bounds::from_corners(point(mid - px(1.), y - px(1.)), point(mid + px(1.), y + px(1.)))
                        });
                    }
                }
            }
            // Characters that can't be seen as themselves (a zero-width space, a direction
            // mark, a no-break space in code): marked, whatever the whitespace setting.
            let mut invisibles = Vec::new();
            let prose = editor.is_prose();
            // Each line's marks, worked out once with all of it (a flag's tags can wrap onto
            // the next row); a very long line's, from the row alone.
            let mut line_marks: std::collections::HashMap<usize, Vec<(usize, crate::editor::invisible::Invisible)>> =
                Default::default();
            for (i, r) in row_layouts.iter().enumerate() {
                if r.row.block.is_some() || r.text.is_ascii() {
                    continue;
                }
                let kinds: Vec<(usize, crate::editor::invisible::Invisible)> =
                    if editor.buffer.line_len(r.row.line) > 10_000 {
                        crate::editor::invisible::marks(&r.text, prose)
                    } else {
                        let line = line_marks.entry(r.row.line).or_insert_with(|| {
                            crate::editor::invisible::marks(&editor.buffer.line_text(r.row.line), prose)
                        });
                        line.iter()
                            .filter(|(at, _)| r.row.cols.contains(at))
                            .map(|&(at, kind)| (at - r.row.cols.start, kind))
                            .collect()
                    };
                if kinds.is_empty() {
                    continue;
                }
                let top = row_top(visible.start + i);
                for (k, &(col, kind)) in kinds.iter().enumerate() {
                    // A run of them with no width (hidden text): one box, where it starts.
                    let run_goes_on = k > 0 && kinds[k - 1] == (col - 1, kind);
                    if run_goes_on && kind != crate::editor::invisible::Invisible::OddSpace {
                        continue;
                    }
                    // Found among the glyphs in any order: text turned around (a direction mark)
                    // is drawn in another order than it's written.
                    let x = origin.x + r.x + glyph_x(r, col);
                    let (y0, y1) = (top + line_height * 0.15, top + line_height * 0.85);
                    // No width of its own: a narrow box where it is; a space, its own width.
                    let (x0, x1) = match kind {
                        crate::editor::invisible::Invisible::OddSpace => (x, x + char_width),
                        _ => (x - px(2.), x + px(2.)),
                    };
                    invisibles.push((Bounds::from_corners(point(x0, y0), point(x1, y1)), kind));
                }
            }
            // The bracket next to the caret and its partner get a thin outline; so do a tag's
            // name with the caret in it and its pair's (HTML, JSX).
            let mut bracket_boxes: Vec<Bounds<Pixels>> = editor
                .matching_brackets()
                .map(|(a, b)| [a, b])
                .into_iter()
                .flatten()
                .flat_map(|offset| range_rects(offset..offset + 1))
                .collect();
            if let Some(names) = tag_pair {
                bracket_boxes.extend(names.into_iter().flat_map(&range_rects));
            }
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
            // Other cursors, and where dragged text would land.
            let extra_carets: Vec<Bounds<Pixels>> = editor
                .extra
                .iter()
                .map(|c| c.selection.head)
                .chain(editor.drop_at)
                .map(|head| editor.buffer.point(head))
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
                    scroll_marks.push(mark(problem.start.0, 6., 4., color));
                }
            }
            let (_, caret_x) = pos(caret_line, caret_col);

            // Caret: glide toward its new position, then blink softly once idle.
            let target = point(caret_x, line_height * caret_row as f32);
            editor.set_lens_hits(lens_hits);
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

            let blinks = cx.global::<Settings>().caret_blink;
            let opacity = if !focused {
                0.35
            } else if gliding || !blinks {
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
            // Vim out of Insert mode: a block over the character, faint enough to read it.
            let caret = Some(if editor.block_caret(cx) {
                (
                    Bounds::new(
                        point(origin.x + visual.x, origin.y + visual.y + (line_height - caret_height) / 2.),
                        size(char_width, caret_height),
                    ),
                    opacity * 0.45,
                )
            } else {
                (
                    Bounds::new(
                        point(origin.x + visual.x - px(1.), origin.y + visual.y + (line_height - caret_height) / 2.),
                        size(px(2.), caret_height),
                    ),
                    opacity,
                )
            });

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
                            let spans = editor.line_spans_to_draw(line, cx.global::<Settings>().bracket_colours);
                            let runs = runs_for(&text, line_byte, &spans, &[], &theme, &font);
                            let (shaped, _) = shape_row(&text, &runs);
                            // Hidden line numbers stay hidden on the pinned lines too.
                            let label = if numbered { (line + 1).to_string() } else { String::new() };
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
                bookmark_dots,
                execution,
                indent_guides,
                line_guide,
                sticky,
                numbers,
                chevrons,
                lines,
                selection,
                whitespace,
                invisibles,
                matches,
                symbol_marks,
                link,
                git_marks,
                assist_band,
                ai_tints,
                veils,
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
        for (dot, kind) in &prepaint.breakpoint_dots {
            let quad = match kind {
                // A log point: a square, as it doesn't stop.
                2 => fill(dot.dilate(px(-1.)), theme.warning).corner_radii(px(1.5)),
                1 => fill(*dot, gpui::transparent_black())
                    .border_widths(px(1.5))
                    .border_color(theme.error)
                    .corner_radii(px(4.)),
                _ => fill(*dot, theme.error).corner_radii(px(4.)),
            };
            window.paint_quad(quad);
        }
        for dot in &prepaint.bookmark_dots {
            window.paint_quad(fill(*dot, theme.caret).corner_radii(dot.size.width / 2.));
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
            for (rect, active) in &prepaint.indent_guides {
                window.paint_quad(fill(*rect, if *active { theme.faint } else { theme.hairline }));
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
            for mark in &prepaint.whitespace {
                window.paint_quad(fill(*mark, theme.faint).corner_radii(px(1.)));
            }
            for (mark, kind) in &prepaint.invisibles {
                let color = match kind {
                    crate::editor::invisible::Invisible::Direction | crate::editor::invisible::Invisible::Hidden => {
                        theme.error
                    }
                    _ => theme.warning.opacity(0.8),
                };
                window.paint_quad(gpui::outline(*mark, color, gpui::BorderStyle::Dashed).corner_radii(px(2.)));
            }
            for (line, origin) in &prepaint.lines {
                line.paint(*origin, line_height, window, cx).ok();
            }
            for rect in &prepaint.veils {
                window.paint_quad(fill(*rect, theme.background.opacity(0.65)));
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

    /// A problem too long for what's left of a wrapped row: cut short, with "…".
    #[test]
    fn a_note_is_cut_to_fit() {
        let note = format!("{BLAME_GAP}unused variable: `unused`");
        assert_eq!(shortened(&note, 100), Some(note.clone()), "room for it");
        let cut = shortened(&note, 20).unwrap();
        assert_eq!(cut.chars().count(), 20);
        assert!(cut.ends_with('…') && cut.starts_with(BLAME_GAP), "{cut:?}");
        assert_eq!(shortened(&note, 6), None, "too little room to say anything: left out");
    }

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
        let (shown, runs, placed) = insert_hints(text, &runs, &hints, |_, len| run(len, &font, gpui::black()));
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
mod problem_notes {
    use super::*;

    fn problem(start: usize, severity: DiagnosticSeverity, message: &str) -> crate::editor::Problem {
        crate::editor::Problem {
            range: start..start + 1,
            start: (0, start),
            end: (0, start + 1),
            severity,
            message: message.into(),
            diagnostic: Default::default(),
        }
    }

    /// Unused code fades, a deprecated name is struck through, and a problem's wavy line
    /// still shows over faded code.
    #[test]
    fn marks_fade_strike_and_underline() {
        let theme = Theme::oled();
        let font = gpui::font("Menlo");
        let text = "let unused = old();";
        let marks = [
            (4..10, Mark::Faded),
            (13..16, Mark::Struck),
            (4..10, Mark::Wavy(theme.warning)),
            (0..3, Mark::Faded),
            (4..10, Mark::Faded),
        ];
        let runs = runs_for(text, 0, &[], &marks, &theme, &font);
        let at = |byte: usize| {
            let mut start = 0;
            runs.iter()
                .find(|r| {
                    start += r.len;
                    byte < start
                })
                .unwrap()
        };
        let plain = theme.syntax(Syntax::Plain);
        assert_eq!(at(5).color, plain.opacity(0.45));
        assert!(at(5).underline.is_some_and(|u| u.wavy && u.color == Some(theme.warning)));
        assert!(at(14).strikethrough.is_some());
        assert_eq!(at(14).color, plain);
        assert_eq!(at(11).color, plain);
        assert!(at(11).underline.is_none() && at(11).strikethrough.is_none());
    }

    #[test]
    fn whitespace_shows_where_settings_say() {
        use crate::settings::ShowWhitespace::{All, Selection, Trailing};
        // "\tlet a = 1;" starting at char 10 of the file, selected from its tab to "a".
        let marks =
            |text, start, selected: &[Range<usize>], shown, ends| whitespace_marks(text, start, selected, shown, ends);
        assert_eq!(marks("\tlet a = 1;", 10, &[10..16], Selection, true), [(0, true), (4, false)]);
        assert_eq!(marks("a b", 0, &[], Selection, true), []);
        assert_eq!(marks("a b c", 0, &[0..2, 3..4], Selection, true), [(1, false), (3, false)]);
        // At line ends: only the spaces left after the text (and the selection's).
        assert_eq!(marks("a b  \t", 0, &[], Trailing, true), [(3, false), (4, false), (5, true)]);
        assert_eq!(marks("a b  ", 0, &[], Trailing, false), [], "a wrapped row that doesn't end the line");
        assert_eq!(marks("    ", 0, &[], Trailing, true).len(), 4, "a line of only spaces");
        assert_eq!(marks("a b ", 0, &[0..2], Trailing, true), [(1, false), (3, false)]);
        // Always: every one.
        assert_eq!(marks("\ta b", 0, &[], All, true), [(0, true), (2, false)]);
    }

    #[test]
    fn the_most_serious_problem_on_the_line_is_said_at_its_end() {
        let problems = [
            problem(3, DiagnosticSeverity::WARNING, "unused variable: `x`"),
            problem(5, DiagnosticSeverity::ERROR, "mismatched types\nexpected `u32`, found `&str`"),
            problem(40, DiagnosticSeverity::ERROR, "on another line"),
            problem(7, DiagnosticSeverity::HINT, "a hint stays out"),
        ];
        assert_eq!(problem_note(&problems, 0..20), Some(("mismatched types".into(), DiagnosticSeverity::ERROR)));
        assert_eq!(problem_note(&problems[..1], 0..20).unwrap().0, "unused variable: `x`");
        assert_eq!(problem_note(&problems[3..], 0..20), None);
        let long = [problem(0, DiagnosticSeverity::ERROR, &"x".repeat(300))];
        assert_eq!(problem_note(&long, 0..5).unwrap().0.chars().count(), PROBLEM_NOTE_CHARS + 1);
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
