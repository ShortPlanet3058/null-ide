//! The same change elsewhere. A word typed over by hand (`count` renamed `total`, letter by
//! letter, or a word pasted over it): once typing stops, the other places in the file the old word still
//! stands get a faint mark beside their line, and the changed line a faint "⇥ 2 more places"
//! at its end. ⇥ from the changed word makes the change at the next place (each its own
//! undo step), and from there the next; Esc lets them be. Any other edit forgets them.

use super::{EditKind, Editor};
use gpui::{App, Context, KeyBinding, Window, actions};
use std::ops::Range;
use std::time::{Duration, Instant};

actions!(repeat_edit, [RepeatEdit, ForgetRepeat]);

/// Registered after the editor's own keys: ⇥ and Esc mean this on the changed word.
pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Editor && repeat_edit");
    cx.bind_keys([KeyBinding::new("tab", RepeatEdit, ctx), KeyBinding::new("escape", ForgetRepeat, ctx)]);
}

/// Files bigger than this (in characters) aren't looked through for the other places.
const LARGEST: usize = 2_000_000;
/// More places than this: the word's a common one changed once, not a name renamed.
const MOST_PLACES: usize = 30;
/// Shown once typing has stopped this long.
const SETTLED: Duration = Duration::from_millis(400);

pub(super) struct Repeat {
    /// The word as it was before it was typed over.
    old: String,
    /// The word now.
    new: String,
    /// Where it is (chars): where it's being typed, then the place last changed.
    at: Range<usize>,
    /// The other places the old word still stands (chars), in the file's order.
    places: Vec<Range<usize>>,
    /// The buffer's version these are of.
    version: u64,
    changed_at: Instant,
    /// Drawn again once typing has stopped, to show it.
    _redraw: Option<gpui::Task<()>>,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Editor {
    /// Before an edit: typing or deleting in a word, with one cursor, keeps (or starts)
    /// following it; anything else forgets it. (`repeat_after_edit` follows.)
    pub(super) fn repeat_before_edit(&mut self, range: &Range<usize>, text: &str, kind: EditKind) {
        self.repeat_follows = false;
        if self.repeating {
            return;
        }
        // (Typing, deleting, or a word pasted.)
        let one_word = (matches!(kind, EditKind::Typing | EditKind::Deleting) || !text.is_empty())
            && self.extra.is_empty()
            && self.batch.is_none()
            && self.buffer.len_chars() <= LARGEST
            && text.chars().all(is_word_char);
        if !one_word {
            self.repeat = None;
            return;
        }
        let added = text.chars().count() as isize - range.len() as isize;
        // In the word followed: still it.
        if let Some(repeat) = &mut self.repeat
            && repeat.at.start <= range.start
            && range.end <= repeat.at.end
        {
            repeat.at.end = (repeat.at.end as isize + added) as usize;
            self.repeat_follows = true;
            return;
        }
        // A word started being changed: as it was.
        let mut start = range.start;
        while start > 0 && self.buffer.char_at(start - 1).is_some_and(is_word_char) {
            start -= 1;
        }
        let mut end = range.end;
        while self.buffer.char_at(end).is_some_and(is_word_char) {
            end += 1;
        }
        let old = self.buffer.slice(start..end);
        // (A word typed where there was none isn't a word changed.)
        if old.chars().count() < 2 || !old.chars().any(char::is_alphabetic) {
            self.repeat = None;
            return;
        }
        self.repeat = Some(Repeat {
            old,
            new: String::new(),
            at: start..(end as isize + added) as usize,
            places: Vec::new(),
            version: 0,
            changed_at: Instant::now(),
            _redraw: None,
        });
        self.repeat_follows = true;
    }

    /// After the text changed: the word followed as it is now, and where the old one still
    /// stands; any other change forgets it.
    pub(super) fn repeat_after_edit(&mut self) {
        if self.repeating {
            return;
        }
        if !std::mem::take(&mut self.repeat_follows) {
            self.repeat = None;
            return;
        }
        let Some(repeat) = &mut self.repeat else { return };
        repeat.new = self.buffer.slice(repeat.at.clone());
        repeat.version = self.buffer.version();
        repeat.changed_at = Instant::now();
        let renamed = repeat.new != repeat.old && repeat.new.chars().any(char::is_alphabetic);
        repeat.places = if renamed { places_of(&self.buffer.to_string(), &repeat.old, &repeat.at) } else { Vec::new() };
        if repeat.places.len() > MOST_PLACES {
            repeat.places.clear();
        }
    }

    /// After an edit in the word followed with places to offer: drawn again once typing
    /// stops, when they show.
    pub(super) fn repeat_shown_when_settled(&mut self, cx: &mut Context<Self>) {
        let Some(repeat) = self.repeat.as_mut().filter(|r| !r.places.is_empty()) else { return };
        repeat._redraw = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLED).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        }));
    }

    /// The change on offer: still of this text, places left, typing stopped.
    fn repeat_offered(&self) -> Option<&Repeat> {
        self.repeat
            .as_ref()
            .filter(|r| r.version == self.buffer.version() && !r.places.is_empty() && r.changed_at.elapsed() >= SETTLED)
    }

    /// Whether ⇥ and Esc are the change's: the caret on the word, nothing else open.
    pub(super) fn repeat_key_context(&self, context: &mut gpui::KeyContext) {
        let on_word = self
            .repeat_offered()
            .is_some_and(|r| r.at.contains(&self.selection.head) || r.at.end == self.selection.head);
        if on_word && self.selection.is_empty() && self.completion.is_none() && self.ghost.is_none() {
            context.add("repeat_edit");
        }
    }

    /// The lines of the places left, for their marks.
    pub fn repeat_lines(&self) -> Vec<usize> {
        let Some(repeat) = self.repeat_offered() else { return Vec::new() };
        let mut lines: Vec<usize> = repeat.places.iter().map(|p| self.buffer.point(p.start).0).collect();
        lines.dedup();
        lines
    }

    /// The changed word's line, and what its end says.
    pub fn repeat_tip(&self) -> Option<(usize, String)> {
        let repeat = self.repeat_offered()?;
        let key = if cfg!(target_os = "macos") { "⇥" } else { "Tab" };
        let count = repeat.places.len();
        let places = if count == 1 { "1 more place".to_string() } else { format!("{count} more places") };
        Some((self.buffer.point(repeat.at.start).0, format!("{key} same change in {places}")))
    }

    /// ⇥: the change made at the next place after the caret (round to the first).
    pub(super) fn repeat_edit(&mut self, _: &RepeatEdit, _: &mut Window, cx: &mut Context<Self>) {
        let Some(repeat) = &self.repeat else { return };
        let caret = self.selection.head;
        let ix = repeat.places.iter().position(|p| p.start >= caret).unwrap_or(0);
        let place = repeat.places[ix].clone();
        let new = repeat.new.clone();
        self.repeating = true;
        self.edit(place.clone(), &new, EditKind::Other, cx);
        self.repeating = false;
        let Some(repeat) = &mut self.repeat else { return };
        let added = new.chars().count() as isize - place.len() as isize;
        let moved = |r: &Range<usize>| {
            if r.start > place.start {
                (r.start as isize + added) as usize..(r.end as isize + added) as usize
            } else {
                r.clone()
            }
        };
        repeat.places.remove(ix);
        repeat.places = repeat.places.iter().map(moved).collect();
        repeat.at = place.start..place.start + new.chars().count();
        repeat.version = self.buffer.version();
        if repeat.places.is_empty() {
            self.repeat = None;
        }
        cx.notify();
    }

    pub(super) fn forget_repeat(&mut self, _: &ForgetRepeat, _: &mut Window, cx: &mut Context<Self>) {
        self.repeat = None;
        cx.notify();
    }
}

/// Where `word` stands as a whole word in `text` (chars), but at `skip`.
fn places_of(text: &str, word: &str, skip: &Range<usize>) -> Vec<Range<usize>> {
    let mut places = Vec::new();
    let (mut chars, mut counted) = (0, 0);
    let length = word.chars().count();
    for (byte, _) in text.match_indices(word) {
        let before = text[..byte].chars().next_back();
        let after = text[byte + word.len()..].chars().next();
        if before.is_some_and(is_word_char) || after.is_some_and(is_word_char) {
            continue;
        }
        chars += text[counted..byte].chars().count();
        counted = byte;
        if chars != skip.start {
            places.push(chars..chars + length);
        }
    }
    places
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use gpui::TestAppContext;
    use std::path::PathBuf;

    #[test]
    fn whole_words_but_the_one_changed() {
        let text = "count = recount(count) + count_all + é count";
        assert_eq!(places_of(text, "count", &(0..5)), [16..21, 39..44]);
    }

    #[gpui::test]
    fn a_word_typed_over_is_changed_at_the_other_places_in_turn(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = "let count = 1;\nprint(count);\ncount += recount;\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("x.rs")), cx));
        e.update_in(cx, |e, window, cx| {
            // "count" typed over, letter by letter, as "total".
            e.selection = super::super::Selection { anchor: 4, head: 9 };
            for letter in ["t", "o", "t", "a", "l"] {
                let range = e.selection.range();
                e.edit(range, letter, EditKind::Typing, cx);
            }
            assert_eq!(e.buffer.to_string(), "let total = 1;\nprint(count);\ncount += recount;\n");
            // Offered once typing stops.
            assert!(e.repeat_lines().is_empty());
            e.repeat.as_mut().unwrap().changed_at -= SETTLED;
            assert_eq!(e.repeat_lines(), [1, 2]);
            assert_eq!(e.repeat_tip().map(|t| t.0), Some(0));
            e.repeat_edit(&RepeatEdit, window, cx);
            assert_eq!(e.buffer.to_string(), "let total = 1;\nprint(total);\ncount += recount;\n");
            e.repeat_edit(&RepeatEdit, window, cx);
            assert_eq!(e.buffer.to_string(), "let total = 1;\nprint(total);\ntotal += recount;\n");
            assert!(e.repeat.is_none(), "all done");
            // Each its own undo step.
            e.undo(&super::super::Undo, window, cx);
            assert_eq!(e.buffer.to_string(), "let total = 1;\nprint(total);\ncount += recount;\n");
            // Typing elsewhere forgets an offer.
            e.selection = super::super::Selection { anchor: 4, head: 9 };
            e.edit(4..9, "sum", EditKind::Typing, cx);
            assert!(e.repeat.is_some());
            e.edit(0..0, " ", EditKind::Typing, cx);
            assert!(e.repeat.is_none());
        });
    }
}
