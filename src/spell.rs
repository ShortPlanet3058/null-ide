//! Spelling where text is words: Markdown and text files, and comments in code. The Mac's
//! own checker does it. A line is checked in each of the first languages set in System
//! Settings, and it's in the one it has the fewest mistakes in: "teh" is a word in
//! Indonesian, not in an English sentence, and "fôte" is wrong in a French one. Each line
//! is asked about once. Names, addresses, paths and code aren't words: never marked.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{LazyLock, Mutex};

/// Lines asked about (by a hash of their text), and the byte ranges in them the checker
/// found wrong.
static KNOWN: LazyLock<Mutex<HashMap<u64, Vec<Range<usize>>>>> = LazyLock::new(Default::default);
/// Longer lines aren't checked (a minified file, a data line): they'd be read every frame.
const LONGEST: usize = 2_000;
/// Past this many lines remembered, they're forgotten and asked about again.
const REMEMBERED: usize = 20_000;

/// The byte ranges in `text` of the words worth checking: runs of letters (with an
/// apostrophe inside, as in "don't" or "l’été"), not part of anything that isn't prose: an
/// address, a path, a name_in_code or camelCase, a number, an acronym.
pub fn words(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut at = 0;
    for chunk in text.split(char::is_whitespace) {
        let start = at;
        at += chunk.len() + text[at + chunk.len()..].chars().next().map_or(0, char::len_utf8);
        let trimmed = chunk.trim_matches(|c: char| !c.is_alphanumeric());
        // `In ticks` is code, even in a comment.
        if trimmed.is_empty() || !is_prose(trimmed) || chunk.contains('`') {
            continue;
        }
        let offset = start + chunk.find(trimmed).unwrap_or(0);
        let mut from = 0;
        for part in trimmed.split('-') {
            if is_word(part) {
                found.push(offset + from..offset + from + part.len());
            }
            from += part.len() + 1;
        }
    }
    found
}

/// Whether a chunk between spaces reads as prose, not code, an address or a number.
fn is_prose(chunk: &str) -> bool {
    !chunk.chars().any(|c| c.is_ascii_digit() || "/\\@_=<>{}[]|#$%^&+~`*:.()".contains(c))
}

/// Whether a run is a word to check: letters (an apostrophe inside), at least two of them,
/// not an acronym (HTTP) or a name in code (camelCase, iPhone).
fn is_word(part: &str) -> bool {
    let letters = part.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 2 || !part.chars().all(|c| c.is_alphabetic() || c == '\'' || c == '’') {
        return false;
    }
    !part.chars().skip(1).any(char::is_uppercase)
}

/// What the checker finds wrong in `line`, by byte range (code included: callers keep
/// only the ranges over `words`).
pub fn wrong_in(line: &str) -> Vec<Range<usize>> {
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        line.hash(&mut hasher);
        hasher.finish()
    };
    if let Some(wrong) = KNOWN.lock().ok().and_then(|known| known.get(&key).cloned()) {
        return wrong;
    }
    let wrong = checker::wrong_in(line);
    if let Ok(mut known) = KNOWN.lock() {
        if known.len() > REMEMBERED {
            known.clear();
        }
        known.insert(key, wrong.clone());
    }
    wrong
}

/// The misspelled words of `line`: its `words` (those `keep` says are checked) that the
/// checker finds wrong.
pub fn misspelled_words(line: &str, keep: impl Fn(&Range<usize>) -> bool) -> Vec<Range<usize>> {
    if line.len() > LONGEST {
        return Vec::new();
    }
    let checked: Vec<Range<usize>> = words(line).into_iter().filter(|r| keep(r)).collect();
    if checked.is_empty() {
        return Vec::new();
    }
    let wrong = wrong_in(line);
    checked.into_iter().filter(|r| wrong.iter().any(|w| w.start < r.end && r.start < w.end)).collect()
}

/// UTF-16 units (as the Mac counts) of `text` to its bytes.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn bytes_of(text: &str, units: Range<usize>) -> Range<usize> {
    let (mut unit, mut start, mut end) = (0, text.len(), text.len());
    for (byte, c) in text.char_indices() {
        if unit == units.start {
            start = byte;
        }
        if unit == units.end {
            end = byte;
            break;
        }
        unit += c.len_utf16();
    }
    start..end.max(start)
}

/// What `word` may have been meant to be, likeliest first.
pub fn guesses(word: &str) -> Vec<String> {
    checker::guesses(word)
}

/// `word` is right from now on, here and in every app that uses the Mac's dictionary.
pub fn learn(word: &str) {
    checker::learn(word);
    if let Ok(mut known) = KNOWN.lock() {
        known.clear();
    }
}

#[cfg(all(target_os = "macos", not(test)))]
use mac as checker;

/// The Mac's checker (built in tests too, for the test run by hand).
#[cfg(target_os = "macos")]
#[cfg_attr(test, allow(dead_code))]
mod mac {
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSRange, NSString};

    use std::ops::Range;
    use std::sync::LazyLock;

    /// The languages lines are tried in: the first ones set in System Settings that can
    /// find a mistake at all (one whose dictionary isn't on the Mac finds none, and would
    /// always look best).
    pub(super) static LANGUAGES: LazyLock<Vec<String>> = LazyLock::new(|| {
        let nonsense = "qzxvbn";
        let text = NSString::from_str(nonsense);
        let languages = NSSpellChecker::sharedSpellChecker().userPreferredLanguages();
        languages
            .iter()
            .map(|l| l.to_string())
            .filter(|l| !wrong_in_language(nonsense, &text, Some(&NSString::from_str(l))).is_empty())
            .take(3)
            .collect()
    });

    /// What's wrong in `line` in the language it has the fewest mistakes in.
    pub fn wrong_in(line: &str) -> Vec<Range<usize>> {
        let text = NSString::from_str(line);
        let mut best: Option<Vec<Range<usize>>> = None;
        for language in LANGUAGES.iter() {
            let wrong = wrong_in_language(line, &text, Some(&NSString::from_str(language)));
            if best.as_ref().is_none_or(|b| wrong.len() < b.len()) {
                best = Some(wrong);
            }
        }
        best.unwrap_or_else(|| wrong_in_language(line, &text, None))
    }

    fn wrong_in_language(line: &str, text: &NSString, language: Option<&NSString>) -> Vec<Range<usize>> {
        let checker = NSSpellChecker::sharedSpellChecker();
        let (mut wrong, mut from) = (Vec::new(), 0);
        while from < text.length() {
            // Safety: the word count pointer may be null.
            let found = unsafe {
                checker.checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(
                    text,
                    from as isize,
                    language,
                    false,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if found.length == 0 || found.location < from {
                break;
            }
            wrong.push(super::bytes_of(line, found.location..found.location + found.length));
            from = found.location + found.length;
        }
        wrong
    }

    pub fn guesses(word: &str) -> Vec<String> {
        let word = NSString::from_str(word);
        let range = NSRange::new(0, word.length());
        NSSpellChecker::sharedSpellChecker()
            .guessesForWordRange_inString_language_inSpellDocumentWithTag(range, &word, None, 0)
            .map(|found| found.iter().map(|g| g.to_string()).collect())
            .unwrap_or_default()
    }

    pub fn learn(word: &str) {
        NSSpellChecker::sharedSpellChecker().learnWord(&NSString::from_str(word));
    }
}

/// Elsewhere (for now), nothing is marked.
#[cfg(all(not(target_os = "macos"), not(test)))]
mod checker {
    pub fn wrong_in(_: &str) -> Vec<std::ops::Range<usize>> {
        Vec::new()
    }
    pub fn guesses(_: &str) -> Vec<String> {
        Vec::new()
    }
    pub fn learn(_: &str) {}
}

/// Tests get a small dictionary of their own: the same everywhere, and nothing learned
/// lands in the real one.
#[cfg(test)]
mod checker {
    use std::sync::Mutex;

    static LEARNED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    pub fn wrong_in(line: &str) -> Vec<std::ops::Range<usize>> {
        let wrong = |word: &str| {
            ["teh", "recieve", "speling"].contains(&word.to_lowercase().as_str())
                && !LEARNED.lock().unwrap().iter().any(|w| w == word)
        };
        super::words(line).into_iter().filter(|r| wrong(&line[r.clone()])).collect()
    }

    pub fn guesses(word: &str) -> Vec<String> {
        match word {
            "teh" => vec!["the".into(), "ten".into()],
            "recieve" => vec!["receive".into()],
            _ => Vec::new(),
        }
    }

    pub fn learn(word: &str) {
        LEARNED.lock().unwrap().push(word.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(text: &str) -> Vec<&str> {
        words(text).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn words_are_prose_not_code() {
        assert_eq!(checked("Teh cat, \"sat\" (here)."), ["Teh", "cat", "sat", "here"]);
        assert_eq!(checked("l’été, don't, peut-être"), ["l’été", "don't", "peut", "être"]);
        // Addresses, paths, names in code, numbers and acronyms aren't words.
        assert!(checked("https://x.dev src/main.rs a@b.fr snake_case camelCase HTTP v2 `code` x").is_empty());
        // Byte ranges, after accents.
        let text = "été teh";
        assert_eq!(&text[words(text)[1].clone()], "teh");
    }

    #[test]
    fn misspelled_words_and_their_guesses() {
        let text = "teh `teh` recieve";
        let wrong: Vec<&str> = misspelled_words(text, |_| true).into_iter().map(|r| &text[r]).collect();
        assert_eq!(wrong, ["teh", "recieve"]);
        let wrong = misspelled_words(text, |r| r.start > 3);
        assert_eq!(wrong.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(), ["recieve"]);
        assert_eq!(guesses("teh")[0], "the");
        // The Mac counts in UTF-16: "été 😀 teh" → bytes.
        let text = "été 😀 teh";
        assert_eq!(&text[bytes_of(text, 7..10)], "teh");
    }

    /// The Mac's own checker, for real (run by hand: it asks AppKit).
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn the_macs_checker_answers() {
        use objc2_app_kit::NSSpellChecker;
        use objc2_foundation::NSString;
        let wrong = |w: &str| {
            NSSpellChecker::sharedSpellChecker().checkSpellingOfString_startingAt(&NSString::from_str(w), 0).length > 0
        };
        assert!(wrong("recieve"));
        assert!(!wrong("receive"));
        for word in ["maison", "été", "l’été", "aujourd’hui", "don't", "Teh"] {
            println!("{word}: {}", if wrong(word) { "wrong" } else { "right" });
        }
        println!("languages: {:?}", *super::mac::LANGUAGES);
        // In a sentence, the language is clear: "teh" isn't English.
        let line = "This release fixes teh crash when you recieve a big file.";
        let found: Vec<&str> = super::mac::wrong_in(line).into_iter().map(|r| &line[r]).collect();
        assert_eq!(found, ["teh", "recieve"]);
        let line = "On peut écrire en français aussi : l’été est là, mais pas de fôte.";
        let found: Vec<&str> = super::mac::wrong_in(line).into_iter().map(|r| &line[r]).collect();
        assert_eq!(found, ["fôte"]);
        let line = "Je vais à la maison avec une fôte de frappe.";
        let found: Vec<&str> = super::mac::wrong_in(line).into_iter().map(|r| &line[r]).collect();
        assert_eq!(found, ["fôte"]);
    }
}
