use regex::{Regex, RegexBuilder};
use std::ops::Range;

/// More matches than this aren't highlighted; the count shows "10000+".
pub const MAX_MATCHES: usize = 10_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

impl SearchQuery {
    /// The query as a regex. Plain-text queries are escaped first.
    pub fn build(&self) -> Result<Regex, regex::Error> {
        let mut pattern = if self.regex { self.text.clone() } else { regex::escape(&self.text) };
        if self.whole_word {
            // Half word boundaries: "->" or "foo(" as whole words still match, where \b can't
            // sit next to punctuation.
            pattern = format!(r"\b{{start-half}}(?:{pattern})\b{{end-half}}");
        }
        RegexBuilder::new(&pattern).case_insensitive(!self.case_sensitive).multi_line(true).build()
    }

    /// Byte ranges of every non-empty match in `text`, up to [`MAX_MATCHES`].
    #[cfg(test)]
    pub fn find_all(&self, regex: &Regex, text: &str) -> Vec<Range<usize>> {
        self.find_within(regex, text, None)
    }

    /// Byte ranges of the non-empty matches inside `within` (all of `text` when None), up
    /// to [`MAX_MATCHES`] of those: matches before it don't use up the count.
    pub fn find_within(&self, regex: &Regex, text: &str, within: Option<Range<usize>>) -> Vec<Range<usize>> {
        if self.text.is_empty() {
            return Vec::new();
        }
        let within = within.unwrap_or(0..text.len());
        regex
            .find_iter(text)
            .map(|m| m.range())
            .skip_while(|r| r.start < within.start)
            .take_while(|r| r.end <= within.end)
            .filter(|r| !r.is_empty())
            .take(MAX_MATCHES)
            .collect()
    }

    /// Byte ranges of every non-empty match, however many: for replacing them all.
    pub fn find_every(&self, regex: &Regex, text: &str) -> Vec<Range<usize>> {
        if self.text.is_empty() {
            return Vec::new();
        }
        regex.find_iter(text).filter(|m| !m.is_empty()).map(|m| m.range()).collect()
    }

    /// Whether replacing keeps each match's case: a search that ignores case, for plain
    /// text, with the replacement written all in lower case. Written with capitals, the
    /// replacement goes in as it is.
    /// A replacement that only changes the search's case (`Color` → `color`) is meant as
    /// written: that's the change asked for.
    pub fn keeps_case(&self, replacement: &str) -> bool {
        !self.case_sensitive
            && !self.regex
            && replacement.to_lowercase() != self.text.to_lowercase()
            && replacement.chars().any(char::is_alphabetic)
            && !replacement.chars().any(char::is_uppercase)
    }

    /// The text that replaces the match at `range`. In regex mode `$1`, `${name}`
    /// and `$0` refer to the match's groups, and `\u` `\l` `\U…\E` `\L…\E` change case
    /// (see [`expand_cased`]); otherwise the replacement is literal, in the
    /// match's case when it [keeps case](Self::keeps_case).
    pub fn replacement_for(&self, regex: &Regex, text: &str, range: Range<usize>, replacement: &str) -> String {
        if !self.regex {
            return match text.get(range) {
                Some(matched) if self.keeps_case(replacement) => in_case_of(matched, replacement),
                _ => replacement.to_string(),
            };
        }
        match regex.captures_at(text, range.start).filter(|c| c.get(0).map(|m| m.range()) == Some(range)) {
            Some(caps) => expand_cased(&caps, replacement),
            // Not expected, but never turn a replacement into a deletion.
            None => replacement.to_string(),
        }
    }
}

/// A regex replacement with its groups filled in (`$1`, `${name}`), and its case changed
/// as VS Code and Sublime do: `\u` the next letter upper case, `\l` lower case, `\U` or
/// `\L` everything up to `\E` (or the end). `\u$1` capitalises the first group.
pub fn expand_cased(caps: &regex::Captures, replacement: &str) -> String {
    #[derive(Clone, Copy)]
    enum Case {
        Same,
        Upper,
        Lower,
    }
    let mut out = String::new();
    let mut case = Case::Same;
    // `\u` or `\l` waiting for a letter to change: upper case if true.
    let mut next: Option<bool> = None;
    let mut rest = replacement;
    loop {
        let marker = rest
            .match_indices('\\')
            .find(|(i, _)| rest[i + 1..].starts_with(['u', 'l', 'U', 'L', 'E']))
            .map(|(i, _)| i);
        let piece = &rest[..marker.unwrap_or(rest.len())];
        let mut expanded = String::new();
        caps.expand(piece, &mut expanded);
        let mut expanded = match case {
            Case::Same => expanded,
            Case::Upper => expanded.to_uppercase(),
            Case::Lower => expanded.to_lowercase(),
        };
        if let Some(upper) = next
            && let Some(first) = expanded.chars().next()
        {
            let changed: String = if upper { first.to_uppercase().collect() } else { first.to_lowercase().collect() };
            expanded.replace_range(..first.len_utf8(), &changed);
            next = None;
        }
        out.push_str(&expanded);
        let Some(at) = marker else { break };
        match rest.as_bytes()[at + 1] {
            b'u' => next = Some(true),
            b'l' => next = Some(false),
            b'U' => case = Case::Upper,
            b'L' => case = Case::Lower,
            _ => case = Case::Same,
        }
        rest = &rest[at + 2..];
    }
    out
}

/// `replacement` (all lower case) in the case of `matched`: "FOO" makes it all capitals,
/// "Foo" capitalised, anything else ("foo", "fooBar") as it is.
pub fn in_case_of(matched: &str, replacement: &str) -> String {
    let letters: Vec<char> = matched.chars().filter(|c| c.is_alphabetic()).collect();
    let Some(first) = letters.first() else { return replacement.to_string() };
    let upper = |c: &char| c.is_uppercase();
    if letters.len() > 1 && letters.iter().all(upper) {
        replacement.to_uppercase()
    } else if first.is_uppercase() && !letters[1..].iter().any(upper) {
        let mut chars = replacement.chars();
        chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
    } else {
        replacement.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacements_change_case_as_asked() {
        let regex = regex::Regex::new(r"(\w+)_(\w+)").unwrap();
        let caps = regex.captures("get_name").unwrap();
        assert_eq!(expand_cased(&caps, r"$1\u$2"), "getName");
        assert_eq!(expand_cased(&caps, r"\U$1\E_$2"), "GET_name");
        assert_eq!(expand_cased(&caps, r"\u\L$1"), "Get");
        assert_eq!(expand_cased(&caps, r"${2}_$1"), "name_get", "no marks: as before");
        // A backslash not followed by a mark stays.
        assert_eq!(expand_cased(&caps, r"a\nb"), r"a\nb");
    }

    #[test]
    fn replacing_keeps_the_case_of_each_match() {
        assert_eq!(in_case_of("user", "client"), "client");
        assert_eq!(in_case_of("User", "client"), "Client");
        assert_eq!(in_case_of("USER", "client"), "CLIENT");
        assert_eq!(in_case_of("U", "client"), "Client");
        assert_eq!(in_case_of("userId", "client"), "client", "mixed case: left as written");
        assert_eq!(in_case_of("été", "hiver"), "hiver");
        assert_eq!(in_case_of("Été", "hiver"), "Hiver");
        assert_eq!(in_case_of("42", "x"), "x");
        let q = SearchQuery { text: "user".into(), ..Default::default() };
        assert!(q.keeps_case("client"));
        assert!(!q.keeps_case("Client"), "written with capitals: as it is");
        assert!(!q.keeps_case("42"));
        let to_lower = SearchQuery { text: "Color".into(), ..Default::default() };
        assert!(!to_lower.keeps_case("color"), "changing only the case is what's asked");
        assert!(!SearchQuery { case_sensitive: true, ..q.clone() }.keeps_case("client"));
        assert!(!SearchQuery { regex: true, ..q.clone() }.keeps_case("client"));
        let re = q.build().unwrap();
        let text = "User user USER";
        let all: Vec<String> =
            q.find_all(&re, text).into_iter().map(|r| q.replacement_for(&re, text, r, "client")).collect();
        assert_eq!(all, ["Client", "client", "CLIENT"]);
    }

    /// Matches in a scope count from it: thousands before it don't use up the limit.
    #[test]
    fn matches_within_a_range_count_from_there() {
        let q = SearchQuery { text: ",".into(), ..Default::default() };
        let re = q.build().unwrap();
        let text = ",".repeat(MAX_MATCHES + 5) + "\na, b, c\n";
        let start = MAX_MATCHES + 6;
        assert_eq!(q.find_within(&re, &text, Some(start..text.len())), [start + 1..start + 2, start + 4..start + 5]);
    }

    #[test]
    fn whole_words_can_be_punctuation() {
        let query = SearchQuery { text: "->".into(), whole_word: true, case_sensitive: true, regex: false };
        let regex = query.build().unwrap();
        assert_eq!(query.find_all(&regex, "a -> b").len(), 1);
        let word = SearchQuery { text: "foo".into(), whole_word: true, ..Default::default() };
        let regex = word.build().unwrap();
        assert_eq!(word.find_all(&regex, "foo food (foo)").len(), 2);
    }

    fn query(text: &str) -> SearchQuery {
        SearchQuery { text: text.into(), ..Default::default() }
    }

    #[test]
    fn plain_queries_are_literal_and_ignore_case() {
        let q = query("a.b");
        let re = q.build().unwrap();
        assert_eq!(q.find_all(&re, "A.B axb a.b"), vec![0..3, 8..11]);
    }

    #[test]
    fn options_change_matching() {
        let mut q = query("Line");
        q.case_sensitive = true;
        q.whole_word = true;
        let re = q.build().unwrap();
        assert_eq!(q.find_all(&re, "Line line Lines Line"), vec![0..4, 16..20]);
    }

    #[test]
    fn regex_replacements_expand_groups() {
        let q = SearchQuery { text: r"(\w+)=(\d+)".into(), regex: true, ..Default::default() };
        let re = q.build().unwrap();
        let text = "a=1, b=22";
        let matches = q.find_all(&re, text);
        assert_eq!(matches, vec![0..3, 5..9]);
        assert_eq!(q.replacement_for(&re, text, matches[1].clone(), "$2:$1"), "22:b");
    }

    #[test]
    fn invalid_regex_is_an_error_and_empty_query_finds_nothing() {
        assert!(SearchQuery { text: "(".into(), regex: true, ..Default::default() }.build().is_err());
        let q = query("");
        assert!(q.find_all(&q.build().unwrap(), "anything").is_empty());
    }
}
