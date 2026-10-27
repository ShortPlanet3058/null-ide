//! Spelling where text is words: Markdown and text files, and comments in code. The
//! system's own checker does it: the Mac's, Windows', and on Linux the Hunspell
//! dictionaries the system has (as LibreOffice and Firefox use). A line is checked in each
//! of the person's first languages, and it's in the one it has the fewest mistakes in:
//! "teh" is a word in Indonesian, not in an English sentence, and "fôte" is wrong in a
//! French one. Each line is asked about once. Names, addresses, paths and code aren't
//! words: never marked.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

/// Whether the checker has started (see `warm`): until it has, lines aren't checked,
/// so opening a file never waits for it (the Mac's first use took 48 ms of the first
/// drawing; Linux's dictionaries take longer to read).
static READY: AtomicBool = AtomicBool::new(cfg!(test));

/// Whether there's a spell checker to ask: the Mac's, Windows', or on Linux dictionaries
/// to read.
pub fn available() -> bool {
    #[cfg(all(unix, not(target_os = "macos"), not(test)))]
    return !hunspell::files().is_empty();
    #[allow(unreachable_code)]
    true
}

/// What takes a while to start, off the window's thread: Linux's dictionaries read.
pub fn load() {
    #[cfg(all(unix, not(target_os = "macos"), not(test)))]
    LazyLock::force(&hunspell::DICTIONARIES);
}

/// The checker started (its languages found), once the window is up (after `load`).
pub fn warm() {
    #[cfg(all(target_os = "macos", not(test)))]
    LazyLock::force(&mac::LANGUAGES);
    READY.store(true, Ordering::Relaxed);
}

/// Of the mistakes found in a line in each language, the fewest (see the top).
#[cfg_attr(test, allow(dead_code))]
fn fewest(found: impl Iterator<Item = Vec<Range<usize>>>) -> Option<Vec<Range<usize>>> {
    found.min_by_key(Vec::len)
}

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
    if line.len() > LONGEST || !READY.load(Ordering::Relaxed) {
        return Vec::new();
    }
    let checked: Vec<Range<usize>> = words(line).into_iter().filter(|r| keep(r)).collect();
    if checked.is_empty() {
        return Vec::new();
    }
    let wrong = wrong_in(line);
    checked.into_iter().filter(|r| wrong.iter().any(|w| w.start < r.end && r.start < w.end)).collect()
}

/// UTF-16 units (as the Mac and Windows count) of `text` to its bytes.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
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

/// `word` is right from now on: here and in every app that uses the system's dictionary
/// (the Mac, Windows), or on Linux in Null's own list (`words.txt` in its data folder).
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
        let each = LANGUAGES.iter().map(|l| wrong_in_language(line, &text, Some(&NSString::from_str(l))));
        super::fewest(each).unwrap_or_else(|| wrong_in_language(line, &text, None))
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

#[cfg(all(windows, not(test)))]
use win as checker;

/// Windows' own checker (Settings → Time & language: the languages added there).
#[cfg(windows)]
#[cfg_attr(test, allow(dead_code))]
mod win {
    use std::ops::Range;
    use windows::Win32::Foundation::S_OK;
    use windows::Win32::Globalization::{
        GetUserDefaultLocaleName, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    };
    use windows::core::{HSTRING, PWSTR};

    thread_local! {
        /// This thread's checkers, one a language: made where they're used, as COM wants.
        static CHECKERS: Vec<ISpellChecker> = checkers(&languages());
    }

    /// The person's own language (their regional format's), then English.
    fn languages() -> Vec<String> {
        let mut name = [0u16; 85];
        // Safety: the buffer's length is passed with it.
        let length = unsafe { GetUserDefaultLocaleName(&mut name) };
        let mut tags = Vec::new();
        if length > 1 {
            tags.push(String::from_utf16_lossy(&name[..length as usize - 1]));
        }
        if !tags.iter().any(|t| t.starts_with("en")) {
            tags.push("en-US".into());
        }
        tags
    }

    /// A checker for each of `tags` Windows has a dictionary for.
    pub(super) fn checkers(tags: &[String]) -> Vec<ISpellChecker> {
        // Safety: COM started on this thread (already started on the window's: said, and
        // fine), then plain calls on the factory it gives.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let Ok(factory) =
                CoCreateInstance::<_, ISpellCheckerFactory>(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER)
            else {
                return Vec::new();
            };
            tags.iter()
                .map(|tag| HSTRING::from(tag.as_str()))
                .filter(|tag| factory.IsSupported(tag).is_ok_and(|yes| yes.as_bool()))
                .filter_map(|tag| factory.CreateSpellChecker(&tag).ok())
                .take(3)
                .collect()
        }
    }

    pub(super) fn wrong_in_language(checker: &ISpellChecker, line: &str) -> Vec<Range<usize>> {
        let mut wrong = Vec::new();
        // Safety: COM calls on a checker made on this thread; each error read once.
        unsafe {
            let Ok(errors) = checker.Check(&HSTRING::from(line)) else { return wrong };
            loop {
                let mut error = None;
                if errors.Next(&mut error) != S_OK {
                    break;
                }
                let Some(error) = error else { break };
                if let (Ok(start), Ok(length)) = (error.StartIndex(), error.Length()) {
                    let (start, length) = (start as usize, length as usize);
                    wrong.push(super::bytes_of(line, start..start + length));
                }
            }
        }
        wrong
    }

    pub fn wrong_in(line: &str) -> Vec<Range<usize>> {
        CHECKERS.with(|checkers| super::fewest(checkers.iter().map(|c| wrong_in_language(c, line))).unwrap_or_default())
    }

    pub(super) fn guesses_from(checker: &ISpellChecker, word: &str) -> Vec<String> {
        let mut found = Vec::new();
        // Safety: COM calls on a checker made on this thread; each string given back is
        // copied, then freed as COM says.
        unsafe {
            let Ok(guesses) = checker.Suggest(&HSTRING::from(word)) else { return found };
            loop {
                let mut one = [PWSTR::null()];
                let mut fetched = 0;
                if guesses.Next(&mut one, Some(&mut fetched)) != S_OK || fetched == 0 {
                    break;
                }
                found.extend(one[0].to_string().ok());
                CoTaskMemFree(Some(one[0].0 as *const _));
            }
        }
        found
    }

    /// Guesses from the languages it's wrong in.
    pub fn guesses(word: &str) -> Vec<String> {
        CHECKERS.with(|checkers| {
            let mut found: Vec<String> = Vec::new();
            for checker in checkers.iter().filter(|c| !wrong_in_language(c, word).is_empty()) {
                for guess in guesses_from(checker, word) {
                    if !found.contains(&guess) {
                        found.push(guess);
                    }
                }
            }
            found
        })
    }

    pub fn learn(word: &str) {
        CHECKERS.with(|checkers| {
            for checker in checkers {
                // Safety: a COM call on a checker made on this thread.
                unsafe { checker.Add(&HSTRING::from(word)).ok() };
            }
        });
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(test)))]
use hunspell as checker;

/// Hunspell's dictionaries, as Linux has them (`hunspell-fr`, `hunspell-en-us`…), read by
/// spellbook; and the words learned, in Null's own list.
#[cfg(all(unix, not(target_os = "macos")))]
#[cfg_attr(test, allow(dead_code))]
mod hunspell {
    use spellbook::Dictionary;
    use std::ops::Range;
    use std::path::{Path, PathBuf};
    use std::sync::{LazyLock, Mutex};

    /// Where distributions put them.
    const FOLDERS: &[&str] =
        &["/usr/share/hunspell", "/usr/share/myspell/dicts", "/usr/share/myspell", "/usr/local/share/hunspell"];

    pub(super) static DICTIONARIES: LazyLock<Mutex<Vec<Dictionary>>> = LazyLock::new(|| {
        let mut read: Vec<Dictionary> = files().iter().filter_map(|(aff, dic)| dictionary(aff, dic)).collect();
        for word in learned() {
            for dictionary in &mut read {
                dictionary.add(&word).ok();
            }
        }
        Mutex::new(read)
    });

    /// The languages wanted, from the environment (`LANGUAGE`, `LANG`…: "fr_FR", then
    /// "fr"), then English.
    fn wanted() -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for var in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
            for part in std::env::var(var).unwrap_or_default().split(':') {
                let tag = part.split(['.', '@']).next().unwrap_or_default();
                if tag.is_empty() || tag == "C" || tag == "POSIX" {
                    continue;
                }
                names.push(tag.to_string());
                if let Some((language, _)) = tag.split_once('_') {
                    names.push(language.to_string());
                }
            }
        }
        names.extend(["en_US", "en_GB", "en"].map(String::from));
        let mut unique = Vec::new();
        for name in names {
            if !unique.contains(&name) {
                unique.push(name);
            }
        }
        unique
    }

    /// The dictionaries (.aff and .dic) for the languages wanted, one a language, at most
    /// three, found in `folders`.
    pub(super) fn files_in(folders: &[&Path], wanted: &[String]) -> Vec<(PathBuf, PathBuf)> {
        let mut found: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut languages: Vec<String> = Vec::new();
        for name in wanted {
            let language = name.split('_').next().unwrap_or(name).to_string();
            if languages.contains(&language) {
                continue;
            }
            let pair = folders.iter().map(|f| (f.join(format!("{name}.aff")), f.join(format!("{name}.dic"))));
            if let Some(pair) = pair.into_iter().find(|(aff, dic)| aff.is_file() && dic.is_file()) {
                found.push(pair);
                languages.push(language);
            }
            if found.len() == 3 {
                break;
            }
        }
        found
    }

    pub fn files() -> Vec<(PathBuf, PathBuf)> {
        static FILES: LazyLock<Vec<(PathBuf, PathBuf)>> = LazyLock::new(|| {
            let folders: Vec<&Path> = FOLDERS.iter().map(Path::new).collect();
            files_in(&folders, &wanted())
        });
        FILES.clone()
    }

    /// A dictionary's text: UTF-8, else Latin-1 (older ones say `SET ISO8859-1`).
    fn text(path: &Path) -> Option<String> {
        let bytes = std::fs::read(path).ok()?;
        Some(String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect()))
    }

    pub(super) fn dictionary(aff: &Path, dic: &Path) -> Option<Dictionary> {
        Dictionary::new(&text(aff)?, &text(dic)?).ok()
    }

    fn learned_file() -> Option<PathBuf> {
        Some(crate::tools::data_dir()?.join("words.txt"))
    }

    fn learned() -> Vec<String> {
        let text = learned_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        text.lines().map(str::trim).filter(|w| !w.is_empty()).map(String::from).collect()
    }

    /// Whether `dictionary` has `word`: as written, with a straight apostrophe, or (French's
    /// "l’été") the word after it.
    pub(super) fn knows(dictionary: &Dictionary, word: &str) -> bool {
        let straight = word.replace('’', "'");
        dictionary.check(word)
            || dictionary.check(&straight)
            || straight.rsplit_once('\'').is_some_and(|(_, after)| after.chars().count() > 1 && dictionary.check(after))
    }

    pub(super) fn wrong_in_language(dictionary: &Dictionary, line: &str) -> Vec<Range<usize>> {
        super::words(line).into_iter().filter(|r| !knows(dictionary, &line[r.clone()])).collect()
    }

    pub fn wrong_in(line: &str) -> Vec<Range<usize>> {
        let Ok(dictionaries) = DICTIONARIES.lock() else { return Vec::new() };
        super::fewest(dictionaries.iter().map(|d| wrong_in_language(d, line))).unwrap_or_default()
    }

    /// Guesses from the languages it's wrong in.
    pub fn guesses(word: &str) -> Vec<String> {
        let Ok(dictionaries) = DICTIONARIES.lock() else { return Vec::new() };
        let mut found: Vec<String> = Vec::new();
        for dictionary in dictionaries.iter().filter(|d| !knows(d, word)) {
            let mut guesses = Vec::new();
            dictionary.suggest(word, &mut guesses);
            for guess in guesses {
                if !found.contains(&guess) {
                    found.push(guess);
                }
            }
        }
        found
    }

    pub fn learn(word: &str) {
        if let Ok(mut dictionaries) = DICTIONARIES.lock() {
            for dictionary in dictionaries.iter_mut() {
                dictionary.add(word).ok();
            }
        }
        if let Some(file) = learned_file() {
            use std::io::Write;
            file.parent().map(std::fs::create_dir_all);
            let opened = std::fs::OpenOptions::new().create(true).append(true).open(file);
            opened.and_then(|mut f| writeln!(f, "{word}")).ok();
        }
    }
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

    /// Windows' own checker, for real (CI's Windows has English).
    #[test]
    #[cfg(windows)]
    fn windows_checker_answers() {
        let checkers = super::win::checkers(&["en-US".to_string()]);
        let Some(english) = checkers.first() else { return eprintln!("no English checker here") };
        let line = "This release fixes teh crash when you recieve a big file.";
        let found: Vec<&str> = super::win::wrong_in_language(english, line).into_iter().map(|r| &line[r]).collect();
        assert_eq!(found, ["teh", "recieve"]);
        assert!(super::win::guesses_from(english, "recieve").contains(&"receive".to_string()));
    }

    /// Linux's: Hunspell dictionaries found where distributions put them, read, and asked.
    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn hunspell_dictionaries_answer() {
        let dir = crate::tools::test_dir("hunspell");
        let aff = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz'\nSFX S Y 1\nSFX S 0 s .\n";
        std::fs::write(dir.join("en_US.aff"), aff).unwrap();
        std::fs::write(dir.join("en_US.dic"), "4\ncat/S\nreceive\nthe\ndon't\n").unwrap();
        std::fs::write(dir.join("fr.aff"), "SET UTF-8\n").unwrap();
        std::fs::write(dir.join("fr.dic"), "2\nété\nl'\n").unwrap();
        let wanted = ["fr_FR", "fr", "en_US", "en"].map(String::from);
        let files = super::hunspell::files_in(&[dir.as_path()], &wanted);
        // One a language, in the order wanted (fr_FR has none: fr's).
        let names: Vec<_> = files.iter().map(|(_, dic)| dic.file_name().unwrap().to_owned()).collect();
        assert_eq!(names, ["fr.dic", "en_US.dic"]);
        let english = super::hunspell::dictionary(&files[1].0, &files[1].1).unwrap();
        let line = "the cats don't recieve teh";
        let found: Vec<&str> =
            super::hunspell::wrong_in_language(&english, line).into_iter().map(|r| &line[r]).collect();
        assert_eq!(found, ["recieve", "teh"]);
        let french = super::hunspell::dictionary(&files[0].0, &files[0].1).unwrap();
        assert!(super::hunspell::knows(&french, "l’été"));
        // The real ones, if this computer has any.
        println!("wanted here: {:?}", super::hunspell::files());
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
