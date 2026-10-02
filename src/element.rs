use crate::editor::{Editor, Layout, ScrollbarLayout};
use crate::fonts::Fonts;
use crate::highlight::{Span, spans_in};
use crate::theme::{Syntax, Theme};
use gpui::{
    App, Bounds, ContentMask, Element, ElementId, ElementInputHandler, Entity, Focusable, Font, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, ShapedLine, Style, TextRun, UnderlineStyle, Window, fill,
    font, point, px, relative, size,
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
    lines: Vec<(ShapedLine, Point<Pixels>)>,
    selection: Vec<Bounds<Pixels>>,
    matches: Vec<(Bounds<Pixels>, bool)>,
    link: Vec<Bounds<Pixels>>,
    git_marks: Vec<(Bounds<Pixels>, Hsla)>,
    assist_band: Option<Bounds<Pixels>>,
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
            let font = font(code_font.clone());
            let text_system = window.text_system().clone();
            let shape = |text: String, runs: &[TextRun]| text_system.shape_line(text.into(), font_size, runs, None);

            let char_width = shape("0".repeat(10), &[run(10, &font, theme.foreground)]).width / 10.;
            let total_lines = editor.buffer.len_lines();
            let digits = total_lines.to_string().len().max(3);
            let gutter_width = char_width * digits as f32 + px(GUTTER_PADDING * 2.);
            let text_bounds =
                Bounds::from_corners(point(bounds.left() + gutter_width, bounds.top()), bounds.bottom_right());
            let viewport_height = f32::from(bounds.size.height);
            let (caret_line, caret_col) = editor.caret_point();

            // Vertical scroll: follow the caret after keyboard moves, then glide toward the target.
            let max_y = total_lines.saturating_sub(1) as f32 * lh;
            if editor.autoscroll {
                let margin = (3. * lh).min(viewport_height / 3.);
                let y = caret_line as f32 * lh + TOP_PADDING;
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

            let first = ((editor.scroll.y - TOP_PADDING) / lh).floor().max(0.) as usize;
            let count = (viewport_height / lh).ceil() as usize + 2;
            let visible = first.min(total_lines)..(first + count).min(total_lines);

            let texts: Vec<String> = visible.clone().map(|l| editor.buffer.line_text(l)).collect();

            // Errors and warnings get a wavy underline and color their line number.
            // Hints and notes only show in the hover card, to keep the code calm.
            let mut underlines: Vec<Vec<(Range<usize>, Hsla)>> = vec![Vec::new(); visible.len()];
            let mut flagged: Vec<Option<Hsla>> = vec![None; visible.len()];
            for problem in editor.problems(cx) {
                let color = match problem.severity {
                    DiagnosticSeverity::ERROR => theme.error,
                    DiagnosticSeverity::WARNING => theme.warning,
                    _ => continue,
                };
                let (start_line, start_col) = editor.buffer.point(problem.range.start);
                let (end_line, end_col) = editor.buffer.point(problem.range.end);
                for line in start_line.max(visible.start)..=end_line.min(visible.end.saturating_sub(1)) {
                    let i = line - visible.start;
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

            let shaped: Vec<ShapedLine> = visible
                .clone()
                .zip(&texts)
                .enumerate()
                .map(|(i, (line, text))| {
                    let runs =
                        runs_for(text, editor.buffer.line_to_byte(line), &editor.spans, &underlines[i], &theme, &font);
                    shape(text.clone(), &runs)
                })
                .collect();
            let x_of = |line: usize, col: usize| -> Pixels {
                if visible.contains(&line) {
                    let i = line - visible.start;
                    shaped[i].x_for_index(byte_of_column(&texts[i], col))
                } else {
                    char_width * col as f32
                }
            };

            // Horizontal scroll: keep the caret in view, never scroll past the longest visible line.
            let text_width = f32::from(text_bounds.size.width) - TEXT_PADDING;
            let caret_x = f32::from(x_of(caret_line, caret_col));
            let cw = f32::from(char_width);
            if editor.autoscroll {
                if caret_x < editor.scroll.x + cw * 2. {
                    editor.scroll.x = caret_x - cw * 4.;
                } else if caret_x > editor.scroll.x + text_width - cw * 4. {
                    editor.scroll.x = caret_x - text_width + cw * 8.;
                }
            }
            let widest = shaped.iter().map(|s| f32::from(s.width)).fold(caret_x, f32::max);
            editor.scroll.x = editor.scroll.x.clamp(0., (widest + cw * 4. - text_width).max(0.));
            editor.autoscroll = false;

            let origin = point(
                text_bounds.left() + px(TEXT_PADDING - editor.scroll.x),
                bounds.top() + px(TOP_PADDING - editor.scroll.y),
            );
            let row_top = |line: usize| origin.y + line_height * line as f32;

            let selection_range = editor.selection.range();
            // While the Cmd+I card is open, the code it changes is tinted, with a bar in the gutter.
            let assist_band =
                editor.assist_target(cx).filter(|l| l.end > visible.start && l.start < visible.end).map(|l| {
                    Bounds::from_corners(
                        point(bounds.left() + gutter_width - px(9.), row_top(l.start.max(visible.start))),
                        point(bounds.right(), row_top(l.end.min(visible.end))),
                    )
                });
            let current_line = editor
                .selection
                .is_empty()
                .then(|| Bounds::new(point(bounds.left(), row_top(caret_line)), size(bounds.size.width, line_height)));

            let numbers = visible
                .clone()
                .map(|line| {
                    let label = (line + 1).to_string();
                    let color = flagged[line - visible.start].unwrap_or(if line == caret_line {
                        theme.muted
                    } else {
                        theme.faint
                    });
                    let shaped = shape(label.clone(), &[run(label.len(), &font, color)]);
                    let x = bounds.left() + gutter_width - px(GUTTER_PADDING) - shaped.width;
                    (shaped, point(x, row_top(line)))
                })
                .collect();

            // Rectangles covering a char range on the visible lines, one per line.
            let range_rects = |range: std::ops::Range<usize>| -> Vec<Bounds<Pixels>> {
                let mut rects = Vec::new();
                if range.is_empty() || visible.is_empty() {
                    return rects;
                }
                let (start_line, start_col) = editor.buffer.point(range.start);
                let (end_line, end_col) = editor.buffer.point(range.end);
                for line in start_line.max(visible.start)..=end_line.min(visible.end - 1) {
                    let x0 = if line == start_line { x_of(line, start_col) } else { px(0.) };
                    let x1 = if line == end_line {
                        x_of(line, end_col)
                    } else {
                        shaped[line - visible.start].width + char_width * 0.6
                    };
                    if x1 > x0 {
                        rects.push(Bounds::from_corners(
                            point(origin.x + x0, row_top(line)),
                            point(origin.x + x1, row_top(line) + line_height),
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
            // A thin underline under the word that Cmd/Ctrl+click would follow.
            let link: Vec<Bounds<Pixels>> = editor
                .link_word
                .clone()
                .map(range_rects)
                .unwrap_or_default()
                .into_iter()
                .map(|r| Bounds::new(point(r.left(), r.bottom() - line_height * 0.18), size(r.size.width, px(1.))))
                .collect();

            // Search matches on screen; the current one is drawn with an outline.
            // Changes since the last commit: a bar beside the line numbers, or a small
            // notch between lines where something was deleted.
            let marker_x = bounds.left() + gutter_width - px(9.);
            let git_marks: Vec<(Bounds<Pixels>, Hsla)> = editor
                .git_hunks
                .iter()
                .filter(|h| h.lines.end >= visible.start && h.lines.start <= visible.end)
                .map(|h| match h.change {
                    crate::git::Change::Deleted => (
                        Bounds::new(point(marker_x - px(2.), row_top(h.lines.start) - px(1.5)), size(px(7.), px(3.))),
                        theme.git_deleted,
                    ),
                    change => {
                        let color =
                            if change == crate::git::Change::Added { theme.git_added } else { theme.git_modified };
                        let top = row_top(h.lines.start.max(visible.start));
                        let bottom = row_top(h.lines.end.min(visible.end));
                        (
                            Bounds::from_corners(
                                point(marker_x, top + px(2.)),
                                point(marker_x + px(3.), bottom - px(2.)),
                            ),
                            color,
                        )
                    }
                })
                .collect();

            let mut matches = Vec::new();
            if let Some(search) = &editor.search {
                let first_char = editor.buffer.line_to_char(visible.start);
                let last_char = editor.buffer.line_to_char(visible.end);
                let first = search.matches.partition_point(|m| m.end <= first_char);
                for (i, m) in search.matches.iter().enumerate().skip(first) {
                    if m.start > last_char {
                        break;
                    }
                    let current = search.current == Some(i);
                    matches.extend(range_rects(m.clone()).into_iter().map(|r| (r, current)));
                }
            }

            let mut marked = Vec::new();
            if let Some(range) = editor.marked.clone() {
                let (line, start_col) = editor.buffer.point(range.start);
                let (_, end_col) = editor.buffer.point(range.end);
                if visible.contains(&line) {
                    let x0 = x_of(line, start_col);
                    let x1 = x_of(line, end_col);
                    let y = row_top(line) + line_height * 0.85;
                    marked.push(Bounds::new(point(origin.x + x0, y), size(x1 - x0, px(1.))));
                }
            }

            // Caret: glide toward its new position, then blink softly once idle.
            let target = point(px(caret_x), line_height * caret_line as f32);
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
            let caret_height = line_height * 0.8;
            let extra_carets = editor
                .extra
                .iter()
                .map(|c| editor.buffer.point(c.selection.head))
                .filter(|(line, _)| visible.contains(line))
                .map(|(line, col)| {
                    Bounds::new(
                        point(origin.x + x_of(line, col) - px(1.), row_top(line) + (line_height - caret_height) / 2.),
                        size(px(2.), caret_height),
                    )
                })
                .collect();
            let caret = Some((
                Bounds::new(
                    point(origin.x + visual.x - px(1.), origin.y + visual.y + (line_height - caret_height) / 2.),
                    size(px(2.), caret_height),
                ),
                opacity,
            ));

            let lines = visible
                .clone()
                .zip(shaped.iter().cloned())
                .map(|(line, s)| (s, point(origin.x, row_top(line))))
                .collect();
            // Scrollbar: a thin thumb at the right edge, with marks for errors, search
            // matches and changed lines so they can be found in long files.
            const BAR: f32 = 10.;
            let scrollbar = (max_y > 0.).then(|| {
                let track = Bounds::from_corners(point(bounds.right() - px(BAR), bounds.top()), bounds.bottom_right());
                let track_h = f32::from(track.size.height);
                let thumb_h = (track_h * viewport_height / (viewport_height + max_y)).max(28.).min(track_h);
                let thumb_top = track.top() + px((editor.scroll.y / max_y) * (track_h - thumb_h));
                let thumb = Bounds::new(point(track.left(), thumb_top), size(px(BAR), px(thumb_h)));
                ScrollbarLayout { track, thumb, max_scroll: max_y }
            });
            let mut scroll_marks: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
            if let Some(bar) = &scrollbar {
                let total = total_lines.max(1) as f32;
                let mark = |line: usize, x: f32, w: f32, color: Hsla| {
                    let y = bar.track.top() + bar.track.size.height * (line as f32 / total);
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
                for problem in editor.problems(cx) {
                    let color = match problem.severity {
                        DiagnosticSeverity::ERROR => theme.error,
                        DiagnosticSeverity::WARNING => theme.warning,
                        _ => continue,
                    };
                    scroll_marks.push(mark(editor.buffer.point(problem.range.start).0, 6., 4., color));
                }
            }
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
                line_height,
                char_width,
                visible_lines: visible,
                shaped,
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
                lines,
                selection,
                matches,
                link,
                git_marks,
                assist_band,
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
        for (rect, color) in &prepaint.git_marks {
            window.paint_quad(fill(*rect, *color).corner_radii(px(1.5)));
        }
        for (number, origin) in &prepaint.numbers {
            number.paint(*origin, line_height, window, cx).ok();
        }
        window.with_content_mask(Some(ContentMask { bounds: prepaint.text_bounds }), |window| {
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
