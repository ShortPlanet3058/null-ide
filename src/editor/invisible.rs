//! Characters that can't be seen, or look like others: a zero-width space pasted from a
//! web page, a direction mark that makes code read differently from how it runs ("Trojan
//! Source"), a no-break space where a space was meant. Marked in the code, named in the
//! status bar when the caret is on one, and taken out on asking.

use super::Editor;
use gpui::{Context, Window, actions};

actions!(invisible, [RemoveInvisibleCharacters]);

/// What kind of character one is that can't be seen as itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invisible {
    /// Takes no room at all.
    ZeroWidth,
    /// Changes the direction text is shown in.
    Direction,
    /// A space that isn't the usual one.
    OddSpace,
}

/// Whether `c` is a character that can't be seen as itself, and which kind. (Joiners stay
/// out: emoji and some scripts are written with them.)
pub fn invisible(c: char) -> Option<Invisible> {
    match c {
        '\u{200B}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}' | '\u{180E}' | '\u{034F}' | '\u{2061}'..='\u{2064}' => {
            Some(Invisible::ZeroWidth)
        }
        // Hangul fillers: blank, and allowed in names (the "invisible variable" trick).
        '\u{3164}' | '\u{115F}' | '\u{1160}' | '\u{FFA0}' => Some(Invisible::ZeroWidth),
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}' => {
            Some(Invisible::Direction)
        }
        '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{1680}' => {
            Some(Invisible::OddSpace)
        }
        _ => None,
    }
}

/// Its name, for the status bar.
pub fn name(c: char) -> &'static str {
    match c {
        '\u{200B}' => "zero-width space",
        '\u{2060}' => "word joiner",
        '\u{FEFF}' => "zero-width no-break space",
        '\u{00AD}' => "soft hyphen",
        '\u{034F}' => "combining grapheme joiner",
        '\u{2061}'..='\u{2064}' => "invisible operator",
        '\u{3164}' | '\u{115F}' | '\u{1160}' | '\u{FFA0}' => "Hangul filler",
        '\u{180E}' => "Mongolian vowel separator",
        '\u{202A}' => "left-to-right embedding",
        '\u{202B}' => "right-to-left embedding",
        '\u{202C}' => "pop directional formatting",
        '\u{202D}' => "left-to-right override",
        '\u{202E}' => "right-to-left override",
        '\u{2066}' => "left-to-right isolate",
        '\u{2067}' => "right-to-left isolate",
        '\u{2068}' => "first strong isolate",
        '\u{2069}' => "pop directional isolate",
        '\u{200E}' => "left-to-right mark",
        '\u{200F}' => "right-to-left mark",
        '\u{061C}' => "Arabic letter mark",
        '\u{00A0}' => "no-break space",
        '\u{202F}' => "narrow no-break space",
        '\u{3000}' => "ideographic space",
        _ => "unusual space",
    }
}

/// Whether `c` is marked in a file that `prose` or not: odd spaces are how prose is
/// written (a French "?" has one before it), and so are the marks that say which way a
/// right-to-left language's words go; not code.
pub fn marked(c: char, prose: bool) -> Option<Invisible> {
    let written_so = matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}');
    invisible(c).filter(|kind| !(prose && (*kind == Invisible::OddSpace || written_so)))
}

impl Editor {
    /// The character at the caret (or just before it) that can't be seen as itself: its
    /// code ("U+200B") and name, for the status bar.
    pub fn invisible_at_caret(&self) -> Option<String> {
        let head = self.selection.head;
        let prose = self.is_prose();
        [self.buffer.char_at(head), head.checked_sub(1).and_then(|i| self.buffer.char_at(i))]
            .into_iter()
            .flatten()
            .find(|c| marked(*c, prose).is_some())
            .map(|c| format!("U+{:04X} {}", c as u32, name(c)))
    }

    /// Takes out the characters that can't be seen (in the selection, or the whole file):
    /// those with no width and direction marks go, odd spaces become spaces (in code).
    pub(super) fn remove_invisible_characters(
        &mut self,
        _: &RemoveInvisibleCharacters,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every selection (each cursor's), or the whole file when none selects anything.
        let selected: Vec<std::ops::Range<usize>> =
            self.all_selections().iter().map(|s| s.range()).filter(|r| !r.is_empty()).collect();
        let ranges = if selected.is_empty() { vec![0..self.buffer.len_chars()] } else { selected };
        let prose = self.is_prose();
        let mut edits = Vec::new();
        for range in &ranges {
            let text = self.buffer.slice(range.clone());
            for (i, c) in text.chars().enumerate() {
                match marked(c, prose) {
                    Some(Invisible::OddSpace) => edits.push((range.start + i..range.start + i + 1, " ".to_string())),
                    Some(_) => edits.push((range.start + i..range.start + i + 1, String::new())),
                    None => {}
                }
            }
        }
        // The caret stays by the same text: less what's taken out before it.
        let head = self.selection.head;
        let gone_before = edits.iter().filter(|(r, with)| with.is_empty() && r.end <= head).count();
        let count = edits.len();
        let message = match count {
            0 => "No invisible characters here".to_string(),
            1 => "1 invisible character taken out".to_string(),
            n => format!("{n} invisible characters taken out"),
        };
        if count > 0 {
            self.apply_char_edits(edits, cx);
            self.selection = super::Selection::caret(head - gone_before);
        }
        self.show_notice(self.selection.head, message, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::fonts::Fonts;
    use crate::settings::Settings;
    use crate::theme::Theme;
    use gpui::TestAppContext;
    use std::path::PathBuf;

    #[test]
    fn what_can_t_be_seen_is_told() {
        assert_eq!(invisible('\u{200B}'), Some(Invisible::ZeroWidth));
        assert_eq!(invisible('\u{202E}'), Some(Invisible::Direction));
        assert_eq!(invisible('\u{00A0}'), Some(Invisible::OddSpace));
        assert_eq!(invisible('\u{200D}'), None, "joiners make emoji");
        assert_eq!(invisible('\u{3164}'), Some(Invisible::ZeroWidth), "a blank Hangul filler, allowed in names");
        assert_eq!(marked('\u{200F}', true), None, "right-to-left prose writes them");
        assert_eq!(marked('\u{200F}', false), Some(Invisible::Direction));
        assert_eq!(invisible(' '), None);
        assert_eq!(invisible('é'), None);
        // Prose has its odd spaces (French: "Quoi ?"), not its zero-width ones.
        assert_eq!(marked('\u{202F}', true), None);
        assert_eq!(marked('\u{200B}', true), Some(Invisible::ZeroWidth));
    }

    #[gpui::test]
    fn they_are_named_and_taken_out(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = "let\u{00A0}a = 1;\u{200B}\nif admin\u{202E} { }\n";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(text), Some(PathBuf::from("a.rs")), cx));
        e.update_in(cx, |e, window, cx| {
            e.selection = crate::editor::Selection::caret(3);
            assert_eq!(e.invisible_at_caret().as_deref(), Some("U+00A0 no-break space"));
            e.selection = crate::editor::Selection::caret(1);
            assert_eq!(e.invisible_at_caret(), None);
            e.remove_invisible_characters(&RemoveInvisibleCharacters, window, cx);
            assert_eq!(e.buffer.to_string(), "let a = 1;\nif admin { }\n");
            assert_eq!(e.selection.head, 1, "the caret by the same text");
            // One step back.
            e.undo(&crate::editor::Undo, window, cx);
            assert_eq!(e.buffer.to_string(), text);
        });
    }
}
