//! Smart quotes and dashes, for writing in Markdown and text: a typed `"` or `'` becomes the
//! opening or closing quote it stands for, in the style set on the Mac (“ ” ‘ ’ or « » …), and
//! a second hyphen after a word becomes an em dash. Never in code: not in `ticks`, not inside
//! an HTML tag, and not a line of dashes (`---` stays a rule, `|---|` a table).

use std::sync::LazyLock;

/// The quotes written: double opening and closing, single opening and closing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quotes {
    pub double: (char, char),
    pub single: (char, char),
}

/// The Mac's quotes (Keyboard › Text Input › Use smart quotes), or English curly ones.
pub static QUOTES: LazyLock<Quotes> = LazyLock::new(|| {
    #[cfg(target_os = "macos")]
    if let Some(quotes) = mac_quotes() {
        return quotes;
    }
    Quotes { double: ('“', '”'), single: ('‘', '’') }
});

#[cfg(target_os = "macos")]
fn mac_quotes() -> Option<Quotes> {
    use objc2_foundation::{NSString, NSUserDefaults};
    let defaults = NSUserDefaults::standardUserDefaults();
    let array = defaults.stringArrayForKey(&NSString::from_str("NSUserQuotesArray"))?;
    let chars: Vec<char> = array.iter().filter_map(|s| s.to_string().chars().next()).collect();
    match chars[..] {
        [a, b, c, d] => Some(Quotes { double: (a, b), single: (c, d) }),
        _ => None,
    }
}

/// What typing `c` does, given the text before it on the line.
#[derive(Debug, PartialEq)]
pub enum Smart {
    /// Writes this instead.
    Write(char),
    /// Takes the hyphen before the caret too, and writes this in its place.
    Join(char),
}

/// What typing `c` after `before` (the line up to the caret) writes, if not `c` itself.
pub fn smart(c: char, before: &str, quotes: Quotes) -> Option<Smart> {
    if in_code(before) {
        return None;
    }
    let previous = before.chars().next_back();
    match c {
        '"' | '\'' => {
            let (open, close) = if c == '"' { quotes.double } else { quotes.single };
            // Opening at the start, after a space, a bracket, a tag, a dash or an opening quote.
            let opens = previous.is_none_or(|p| {
                p.is_whitespace()
                    || matches!(p, '(' | '[' | '{' | '<' | '>' | '—' | '–' | '-' | '/')
                    || [quotes.double.0, quotes.single.0].contains(&p)
            });
            Some(Smart::Write(if opens { open } else { close }))
        }
        '-' => {
            let rest = before.strip_suffix('-')?;
            // After a word, not a line of dashes or a table.
            let after_words = rest.chars().next_back().is_some_and(|p| p != '-') && !rest.trim().is_empty();
            (after_words && !before.contains('|')).then_some(Smart::Join('—'))
        }
        _ => None,
    }
}

/// Whether the caret, after `before`, is in code: inside `ticks` or an HTML tag.
fn in_code(before: &str) -> bool {
    let ticks = before.matches('`').count();
    let in_tag = before.rfind('<').is_some_and(|open| before.rfind('>').is_none_or(|close| close < open));
    ticks % 2 == 1 || in_tag
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURLY: Quotes = Quotes { double: ('“', '”'), single: ('‘', '’') };

    fn typed(text: &str) -> String {
        let mut line = String::new();
        for c in text.chars() {
            match smart(c, &line, CURLY) {
                Some(Smart::Write(w)) => line.push(w),
                Some(Smart::Join(w)) => {
                    line.pop();
                    line.push(w);
                }
                None => line.push(c),
            }
        }
        line
    }

    #[test]
    fn quotes_open_and_close() {
        assert_eq!(typed(r#"She said "it's 'fine'" (and "left")."#), "She said “it’s ‘fine’” (and “left”).");
        assert_eq!(typed(r#""Quoted" at the start"#), "“Quoted” at the start");
        assert_eq!(typed("rock 'n' roll, the '90s"), "rock ‘n’ roll, the ‘90s");
        let french = Quotes { double: ('«', '»'), single: ('‹', '›') };
        assert_eq!(smart('"', "Il a dit ", french), Some(Smart::Write('«')));
        assert_eq!(smart('"', "Il a dit «bonjour", french), Some(Smart::Write('»')));
    }

    #[test]
    fn two_hyphens_after_a_word_make_a_dash() {
        assert_eq!(typed("one--two"), "one—two");
        assert_eq!(typed("one -- two"), "one — two");
        assert_eq!(typed("well-known"), "well-known");
        // Lines of dashes stay: a rule, front matter, a table, a list item.
        assert_eq!(typed("---"), "---");
        assert_eq!(typed("  ---"), "  ---");
        assert_eq!(typed("|---|---|"), "|---|---|");
        assert_eq!(typed("- item"), "- item");
    }

    #[test]
    fn code_is_left_alone() {
        assert_eq!(typed(r#"Run `say "hi" --loud` then "go""#), r#"Run `say "hi" --loud` then “go”"#);
        assert_eq!(typed(r#"<a href="x.md">"link"</a>"#), r#"<a href="x.md">“link”</a>"#);
    }

    /// The Mac's quotes are read (run by hand: they're the Mac's own setting).
    #[test]
    #[ignore]
    fn the_macs_quotes() {
        println!("{:?}", *QUOTES);
    }
}
