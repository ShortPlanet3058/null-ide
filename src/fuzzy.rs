/// Fuzzy matching for the command palette: the query's characters must appear
/// in order (ignoring case), and matches that start words or run together
/// score higher.
///
/// Returns the score and the byte offsets of the matched characters, for
/// highlighting. Spaces in the query are ignored.
pub fn score(candidate: &str, query: &str) -> Option<(i32, Vec<usize>)> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<(usize, char)> = candidate.char_indices().collect();
    let lower: Vec<char> = chars.iter().map(|&(_, c)| c.to_lowercase().next().unwrap_or(c)).collect();
    let starts_word = |i: usize| match i.checked_sub(1).map(|j| chars[j].1) {
        None => true,
        Some(p) => {
            matches!(p, '/' | '\\' | '_' | '-' | '.' | ' ' | ':') || (p.is_lowercase() && chars[i].1.is_uppercase())
        }
    };
    let fits_after = |from: usize, rest: &[char]| {
        let mut r = rest.iter().peekable();
        for &c in &lower[from..] {
            if r.peek() == Some(&&c) {
                r.next();
            }
        }
        r.peek().is_none()
    };

    let mut positions = Vec::with_capacity(query.len());
    let mut score = 0;
    let mut from = 0;
    let mut last: Option<usize> = None;
    for (qi, &q) in query.iter().enumerate() {
        let next = (from..chars.len()).find(|&i| lower[i] == q)?;
        // Continuing a run beats jumping; otherwise prefer the start of a word
        // when the rest of the query still fits after it.
        let i = if last.is_some_and(|l| l + 1 == next) || starts_word(next) {
            next
        } else {
            (next + 1..chars.len())
                .find(|&i| lower[i] == q && starts_word(i) && fits_after(i + 1, &query[qi + 1..]))
                .unwrap_or(next)
        };
        score += 1;
        if starts_word(i) {
            score += 8;
        }
        match last {
            Some(l) if l + 1 == i => score += 5,
            Some(l) => score -= ((i - l - 1) as i32).min(5),
            None => score -= (i as i32).min(10),
        }
        positions.push(chars[i].0);
        last = Some(i);
        from = i + 1;
    }
    Some((score, positions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_in_order_ignoring_case_and_spaces() {
        assert!(score("src/editor.rs", "edrs").is_some());
        assert!(score("Toggle Sidebar", "tog side").is_some());
        assert!(score("src/editor.rs", "rse").is_none());
    }

    #[test]
    fn prefers_word_starts_and_runs() {
        let tight = score("buffer.rs", "buf").unwrap().0;
        let loose = score("bad_unused_file.rs", "buf").unwrap().0;
        assert!(tight > loose);
        let boundary = score("next_tab", "t").unwrap().0;
        let inner = score("nextab", "t").unwrap().0;
        assert!(boundary > inner);
    }

    #[test]
    fn reports_byte_positions() {
        assert_eq!(score("a/bc", "ab").unwrap().1, vec![0, 2]);
        assert_eq!(score("é/x", "x").unwrap().1, vec![3]);
    }
}
