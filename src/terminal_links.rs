//! What ⌘-click opens in the terminal's output: a web address, or a place in a file the
//! way compilers, test runners and stack traces print it (`src/a.rs:12:5`,
//! `File "app.py", line 12`, `(/x/app.js:10:5)`).

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Url(String),
    /// A path as printed (relative or not), and the 1-based line and column after it.
    File {
        path: String,
        line: Option<u32>,
        column: Option<u32>,
    },
}

/// The link covering column `at` of a terminal row (one char per column), with the
/// columns it covers.
pub fn link_at(row: &[char], at: usize) -> Option<(Range<usize>, Link)> {
    if at >= row.len() {
        return None;
    }
    // Python: File "app/models.py", line 12
    let text: String = row.iter().collect();
    if let Some(found) = python_place(&text, at) {
        return Some(found);
    }
    // The word under the mouse: anything up to spaces, quotes and brackets.
    let stops = |c: char| {
        c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '<' | '>' | '{' | '}' | '|')
    };
    if stops(row[at]) {
        return None;
    }
    let mut start = at;
    while start > 0 && !stops(row[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < row.len() && !stops(row[end]) {
        end += 1;
    }
    // Punctuation that ends a sentence or a list, not the link.
    while end > start && matches!(row[end - 1], '.' | ',' | ';' | '!' | '?') {
        end -= 1;
    }
    if at >= end {
        return None;
    }
    let word: String = row[start..end].iter().collect();
    if word.starts_with("http://") || word.starts_with("https://") {
        let word = word.trim_end_matches(':');
        return Some((start..start + word.chars().count(), Link::Url(word.to_string())));
    }
    file_place(&word).map(|(len, link)| (start..start + len, link))
}

/// `path`, `path:12` or `path:12:5` (a trailing colon, as Go and grep print, left out):
/// what of `word` it covers, in chars, and the place.
fn file_place(word: &str) -> Option<(usize, Link)> {
    let word = word.trim_end_matches(':');
    let mut parts: Vec<&str> = word.split(':').collect();
    let mut numbers = Vec::new();
    while parts.len() > 1 && parts.last().is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        numbers.insert(0, parts.pop()?.parse::<u32>().ok()?);
    }
    let path = parts.join(":");
    // Looks like a file: has a folder in it, or a name with an extension made of letters.
    let name = path.rsplit('/').next().unwrap_or(&path);
    let has_extension = name
        .rsplit_once('.')
        .is_some_and(|(stem, ext)| (!stem.is_empty() || ext.len() > 1) && ext.chars().any(|c| c.is_ascii_alphabetic()));
    if path.is_empty() || !(path.contains('/') || has_extension) || path.contains("://") {
        return None;
    }
    let len = word.chars().count();
    Some((len, Link::File { path, line: numbers.first().copied(), column: numbers.get(1).copied() }))
}

fn python_place(text: &str, at: usize) -> Option<(Range<usize>, Link)> {
    let start = text.find("File \"")?;
    let path_start = start + "File \"".len();
    let path_end = path_start + text[path_start..].find('"')?;
    let rest = text[path_end..].strip_prefix("\", line ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let line: u32 = digits.parse().ok()?;
    let chars = |byte: usize| text[..byte].chars().count();
    let range = chars(path_start)..chars(path_end + "\", line ".len() + digits.len());
    range
        .contains(&at)
        .then(|| (range, Link::File { path: text[path_start..path_end].to_string(), line: Some(line), column: None }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, needle: &str) -> Option<Link> {
        let chars: Vec<char> = text.chars().collect();
        let column = text[..text.find(needle).unwrap()].chars().count();
        link_at(&chars, column).map(|(_, link)| link)
    }

    fn file(path: &str, line: Option<u32>, column: Option<u32>) -> Option<Link> {
        Some(Link::File { path: path.into(), line, column })
    }

    #[test]
    fn finds_places_as_tools_print_them() {
        assert_eq!(
            at("  --> src/editor/conflicts.rs:283:13", "conflicts"),
            file("src/editor/conflicts.rs", Some(283), Some(13))
        );
        assert_eq!(at("panicked at src/a.rs:12:5:", "src"), file("src/a.rs", Some(12), Some(5)));
        assert_eq!(at("cart_test.go:14: want 3, got 2", "cart"), file("cart_test.go", Some(14), None));
        assert_eq!(
            at("    at Object.<anonymous> (/x/app.test.js:10:5)", "app"),
            file("/x/app.test.js", Some(10), Some(5))
        );
        assert_eq!(
            at("  File \"/srv/app/models.py\", line 12, in total", "models"),
            file("/srv/app/models.py", Some(12), None)
        );
        assert_eq!(at("see README.md.", "README"), file("README.md", None, None));
        assert_eq!(
            at("open https://example.com/docs, then", "example"),
            Some(Link::Url("https://example.com/docs".into()))
        );
    }

    #[test]
    fn leaves_plain_words_alone() {
        assert_eq!(at("version 1.5 released", "1.5"), None);
        assert_eq!(at("test result: ok. 5 passed", "ok"), None);
        assert_eq!(at("running 12 tests", "12"), None);
        assert_eq!(at("a b", " "), None);
    }
}
