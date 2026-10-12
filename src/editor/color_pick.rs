//! A click on a color's square opens the Mac's color panel, set to that color. While the
//! panel stays open, the color picked there is written in place of the one in the code,
//! the way it was written (`#f80`, `rgb(…)`, `hsl(…)`), and undoes as one step.

use super::{EditKind, Editor, EditorEvent, Selection};
use crate::color_panel::Panel;
use gpui::{Context, EntityId, Global, Hsla, Pixels, Point, Rgba, Task};
use std::time::Duration;

/// The editor that picked last: the panel is one for them all, so a pick elsewhere stops
/// the one before.
struct LatestPick(EntityId);

impl Global for LatestPick {}

/// How often the open panel is asked for its color.
const FOLLOW_EVERY: Duration = Duration::from_millis(120);

/// A color being picked: where it's written (chars) and how it's written now.
pub(super) struct ColorPick {
    start: usize,
    written: String,
    /// The text before the first color picked has been kept for undo.
    kept_for_undo: bool,
    _follow: Task<()>,
}

/// A color's channels as bytes, to tell two colors apart as they'd be written.
fn bytes(color: Rgba) -> [u8; 4] {
    [color.r, color.g, color.b, color.a].map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
}

impl Editor {
    /// The color whose square is under `position`: where it starts (a char), how it's
    /// written, and what it is.
    pub(super) fn swatch_at(&self, position: Point<Pixels>) -> Option<(usize, String, Hsla)> {
        if !self.language().is_some_and(|l| crate::colors::written_in(l.name)) {
            return None;
        }
        let layout = self.layout.as_ref()?;
        // A line pinned at the top covers the rows under it.
        if layout.sticky.iter().any(|(bounds, _)| bounds.contains(&position)) {
            return None;
        }
        let row = ((position.y - layout.text_origin.y) / layout.line_height).floor();
        if row < 0. {
            return None;
        }
        let r = layout.rows.get((row as usize).checked_sub(layout.first_row)?)?;
        if r.row.block.is_some() {
            return None;
        }
        let x = position.x - layout.text_origin.x - r.x;
        r.gaps.iter().filter(|gap| gap.hint).find_map(|gap| {
            let (color, len) = crate::colors::color_at(&r.text, gap.byte)?;
            let shown = r.shown_byte(gap.byte);
            let over = r.shaped.x_for_index(shown) <= x && x < r.shaped.x_for_index(shown + gap.extra);
            over.then(|| {
                let col = r.row.cols.start + r.text[..gap.byte].chars().count();
                (self.buffer.offset(r.row.line, col), r.text[gap.byte..gap.byte + len].to_string(), color)
            })
        })
    }

    /// Opens the color panel on the color written at `start`, and follows it while it's open.
    pub(super) fn pick_color(&mut self, start: usize, written: String, color: Hsla, cx: &mut Context<Self>) {
        self.single_cursor();
        self.selection = Selection::caret(start);
        crate::color_panel::open(color.into());
        let follow = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(FOLLOW_EVERY).await;
                let following = this.update(cx, |this, cx| this.follow_color(crate::color_panel::state(), cx));
                if !following.unwrap_or(false) {
                    break;
                }
            }
        });
        let me = cx.entity_id();
        cx.set_global(LatestPick(me));
        self.color_pick = Some(ColorPick { start, written, kept_for_undo: false, _follow: follow });
        cx.notify();
    }

    /// The panel now: its color written in place of the one being picked when it's another.
    /// Returns whether to go on following it.
    pub(super) fn follow_color(&mut self, panel: Panel, cx: &mut Context<Self>) -> bool {
        let Some(pick) = &self.color_pick else { return false };
        let len = pick.written.chars().count();
        // Closed, picking another color somewhere else, or the color's text changed some
        // other way: done. Away with Null in the background: waiting for it.
        let latest = cx.try_global::<LatestPick>().is_some_and(|l| l.0 == cx.entity_id());
        let unchanged = self.buffer.slice(pick.start..pick.start + len) == pick.written;
        let color = match panel {
            Panel::Open(color) if latest && unchanged => color,
            Panel::Away if latest && unchanged => return true,
            _ => {
                self.color_pick = None;
                return false;
            }
        };
        let now = crate::colors::color_at(&pick.written, 0).map(|(c, _)| bytes(c.into()));
        if now == Some(bytes(color)) {
            return true;
        }
        let text = crate::colors::written_like(&pick.written, color);
        // Written that way already (a color hsl() can only come near): nothing to write.
        if text == pick.written {
            return true;
        }
        let (start, keep) = (pick.start, !pick.kept_for_undo);
        if keep {
            self.record_undo(EditKind::Other);
        }
        let end = self.buffer.replace(start..start + len, &text);
        // What's after the color moves with it.
        let shift = |o: usize| if o >= start + len { o + end - (start + len) } else { o };
        self.selection = Selection { anchor: shift(self.selection.anchor), head: shift(self.selection.head) };
        if let Some(pick) = &mut self.color_pick {
            pick.written = text;
            pick.kept_for_undo = true;
        }
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::buffer::Buffer;
    use crate::color_panel::Panel;
    use crate::editor::Editor;
    use gpui::{Rgba, TestAppContext, point, px};
    use std::path::PathBuf;

    /// A click on a color's square picks it; colors from the panel are written as the first
    /// was, and one undo takes them all back.
    #[gpui::test]
    fn a_color_is_picked_from_its_square(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = "a { color: #f80; top: 0 }\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("a.css")), cx));
        cx.run_until_parked();
        // Where the square is drawn, from the layout.
        let (on_square, on_text) = e.read_with(cx, |e, _| {
            let layout = e.layout.as_ref().unwrap();
            let r = &layout.rows[0];
            let gap = r.gaps.iter().find(|g| g.hint).unwrap();
            let x = |i: usize| layout.text_origin.x + r.x + r.shaped.x_for_index(i);
            let y = layout.text_origin.y + layout.line_height / 2.;
            let shown = r.shown_byte(gap.byte);
            (point(x(shown) + px(2.), y), point(x(shown + gap.extra) + px(2.), y))
        });
        assert!(e.read_with(cx, |e, _| e.swatch_at(on_text).is_none()), "the color's text isn't its square");
        let (start, written, _) = e.read_with(cx, |e, _| e.swatch_at(on_square)).expect("the square");
        assert_eq!((start, written.as_str()), (11, "#f80"));
        e.update(cx, |e, cx| {
            let (start, written, color) = e.swatch_at(on_square).unwrap();
            e.pick_color(start, written, color, cx);
            // The panel still on the same color: nothing written.
            assert!(e.follow_color(Panel::Open(Rgba { r: 1., g: 136. / 255., b: 0., a: 1. }), cx));
            assert_eq!(e.buffer.to_string(), text);
            assert!(e.follow_color(Panel::Open(Rgba { r: 0., g: 0.5, b: 1., a: 1. }), cx));
            // Away while Null is in the background: still following, nothing written.
            assert!(e.follow_color(Panel::Away, cx));
            assert!(e.follow_color(Panel::Open(Rgba { r: 0., g: 0., b: 1., a: 0.5 }), cx));
            assert_eq!(e.buffer.to_string(), "a { color: #0000ff80; top: 0 }\n");
            // Closed: no more following.
            assert!(!e.follow_color(Panel::Closed, cx));
            e.step_history(true, cx);
            assert_eq!(e.buffer.to_string(), text, "one undo for the whole pick");
        });
    }

    /// A color the notation can't hold exactly (hsl's whole percents) isn't written again
    /// and again; and a pick started elsewhere stops this one.
    #[gpui::test]
    fn a_pick_writes_once_and_gives_way_to_the_next(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = "a { color: hsl(0, 0%, 0%); }\n";
        let (e, cx) =
            cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("a.css")), cx));
        e.update(cx, |e, cx| {
            let (color, _) = crate::colors::color_at("hsl(0, 0%, 0%)", 0).unwrap();
            e.pick_color(11, "hsl(0, 0%, 0%)".into(), color, cx);
            let between = Rgba { r: 0.123, g: 0.456, b: 0.789, a: 1. };
            assert!(e.follow_color(Panel::Open(between), cx));
            let once = e.buffer.to_string();
            let version = e.buffer.version();
            assert!(e.follow_color(Panel::Open(between), cx));
            assert_eq!((e.buffer.to_string(), e.buffer.version()), (once, version), "not written again");
        });
        // A pick in another editor: this one stops following.
        let (other, cx) = cx
            .add_window_view(|_, cx| Editor::new(Buffer::from_text("b { color: #000; }\n"), Some("b.css".into()), cx));
        other.update(cx, |o, cx| o.pick_color(11, "#000".into(), gpui::black(), cx));
        e.update(cx, |e, cx| assert!(!e.follow_color(Panel::Open(Rgba { r: 1., g: 0., b: 0., a: 1. }), cx)));
    }
}
