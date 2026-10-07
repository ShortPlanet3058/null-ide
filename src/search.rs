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

    /// The text that replaces the match at `range`. In regex mode `$1`, `${name}`
    /// and `$0` refer to the match's groups; otherwise the replacement is literal.
    pub fn replacement_for(&self, regex: &Regex, text: &str, range: Range<usize>, replacement: &str) -> String {
        if !self.regex {
            return replacement.to_string();
        }
        match regex.captures_at(text, range.start).filter(|c| c.get(0).map(|m| m.range()) == Some(range)) {
            Some(caps) => {
                let mut out = String::new();
                caps.expand(replacement, &mut out);
                out
            }
            // Not expected, but never turn a replacement into a deletion.
            None => replacement.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
