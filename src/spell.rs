//! Spelling where text is words: Markdown and text files, and comments in code. The Mac's
//! own checker does it, in the languages chosen in System Settings (it tells them apart
//! itself). Each word is asked about once. Names, addresses, paths and code aren't words,
//! so they're never marked.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{LazyLock, Mutex};

/// Words asked about, and whether they were wrong.
static KNOWN: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);

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

/// Whether `word` is spelled wrong.
pub fn misspelled(word: &str) -> bool {
    if let Some(wrong) = KNOWN.lock().ok().and_then(|known| known.get(word).copied()) {
        return wrong;
    }
    let wrong = checker::wrong(word);
    if let Ok(mut known) = KNOWN.lock() {
        known.insert(word.to_string(), wrong);
    }
    wrong
}

/// What `word` may have been meant to be, likeliest first.
pub fn guesses(word: &str) -> Vec<String> {
    checker::guesses(word)
}

/// `word` is right from now on, here and in every app that uses the Mac's dictionary.
pub fn learn(word: &str) {
    checker::learn(word);
    if let Ok(mut known) = KNOWN.lock() {
        known.insert(word.to_string(), false);
    }
}

#[cfg(all(target_os = "macos", not(test)))]
mod checker {
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSRange, NSString};

    pub fn wrong(word: &str) -> bool {
        let word = NSString::from_str(word);
        NSSpellChecker::sharedSpellChecker().checkSpellingOfString_startingAt(&word, 0).length > 0
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
    pub fn wrong(_: &str) -> bool {
        false
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

    pub fn wrong(word: &str) -> bool {
        ["teh", "recieve", "speling"].contains(&word.to_lowercase().as_str())
            && !LEARNED.lock().unwrap().iter().any(|w| w == word)
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
        let wrong: Vec<&str> = words(text).into_iter().map(|r| &text[r]).filter(|w| misspelled(w)).collect();
        assert_eq!(wrong, ["teh", "recieve"]);
        assert_eq!(guesses("teh")[0], "the");
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
    }
}
