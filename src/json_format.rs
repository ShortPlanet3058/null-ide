//! JSON laid out by Null itself, for Format Document when no language server does it: one
//! member a line, indented by the file's own unit, keys in their order, numbers and strings
//! exactly as written, and the comments of `tsconfig.json` and other JSONC kept where they
//! are. A blank line between members stays (one at most).

/// A piece of the text, and how many line breaks came before it.
#[derive(Debug, PartialEq)]
enum Token<'a> {
    Open(char),
    Close(char),
    Comma,
    Colon,
    /// A string, a number, `true`, a name in JSON5…: as written.
    Value(&'a str),
    /// `// …` (`line`) or `/* … */`.
    Comment(&'a str, bool),
}

/// Why the text couldn't be laid out: the line (from 0) and what's wrong.
#[derive(Debug, PartialEq)]
pub struct Problem {
    pub line: usize,
    pub message: String,
}

/// The pieces of `text`, each with the line breaks before it and its line.
fn tokens(text: &str) -> Result<Vec<(Token<'_>, usize, usize)>, Problem> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut breaks = 0;
    // The line `i` is on, counted along.
    let mut line = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        let token = match c {
            b'\n' => {
                breaks += 1;
                line += 1;
                i += 1;
                continue;
            }
            b' ' | b'\t' | b'\r' => {
                i += 1;
                continue;
            }
            b'{' | b'[' => {
                i += 1;
                Token::Open(c as char)
            }
            b'}' | b']' => {
                i += 1;
                Token::Close(c as char)
            }
            b',' => {
                i += 1;
                Token::Comma
            }
            b':' => {
                i += 1;
                Token::Colon
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = text[i..].find('\n').map_or(b.len(), |n| i + n);
                i = end;
                Token::Comment(text[start..end].trim_end(), true)
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let Some(n) = text[i + 2..].find("*/") else {
                    return Err(Problem { line, message: "a comment that never ends".into() });
                };
                i += 2 + n + 2;
                let comment = &text[start..i];
                // Its own lines count, for what comes after it.
                out.push((Token::Comment(comment, false), breaks, line));
                line += comment.matches('\n').count();
                breaks = 0;
                continue;
            }
            b'"' | b'\'' => {
                i += 1;
                loop {
                    match b.get(i) {
                        None | Some(b'\n') => {
                            return Err(Problem { line, message: "a string that never ends".into() });
                        }
                        Some(b'\\') => i += 2,
                        Some(&q) if q == c => {
                            i += 1;
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
                Token::Value(&text[start..i])
            }
            _ => {
                while i < b.len()
                    && !matches!(
                        b[i],
                        b' ' | b'\t' | b'\r' | b'\n' | b'{' | b'}' | b'[' | b']' | b',' | b':' | b'"' | b'\''
                    )
                    && !(b[i] == b'/' && matches!(b.get(i + 1), Some(b'/' | b'*')))
                {
                    i += 1;
                }
                Token::Value(&text[start..i])
            }
        };
        out.push((token, breaks, line));
        breaks = 0;
    }
    Ok(out)
}

/// `text` laid out with `unit` (two spaces, a tab…) and `newline` between lines; its last
/// line break kept if it had one.
pub fn format(text: &str, unit: &str, newline: &str) -> Result<String, Problem> {
    let tokens = tokens(text)?;
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut open: Vec<(char, usize)> = Vec::new();
    // A line break is due before the next thing written (not before a `,` or a comment that
    // follows on the same line).
    let mut break_due = false;
    let mut space_due = false;
    let mut skip_close = false;
    let mut after_line_comment = false;
    let new_line = |out: &mut String, depth: usize, blank: bool| {
        while out.ends_with(' ') {
            out.pop();
        }
        if !out.is_empty() {
            out.push_str(newline);
            if blank {
                out.push_str(newline);
            }
        }
        for _ in 0..depth {
            out.push_str(unit);
        }
    };
    for (n, (token, breaks, token_line)) in tokens.iter().enumerate() {
        let first_in_block = matches!(n.checked_sub(1).map(|p| &tokens[p].0), Some(Token::Open(_)));
        let follows_line_comment = std::mem::replace(&mut after_line_comment, matches!(token, Token::Comment(_, true)));
        let blank = *breaks >= 2 && !first_in_block;
        match token {
            Token::Close(c) => {
                if skip_close {
                    skip_close = false;
                    continue;
                }
                let wanted = if *c == '}' { '{' } else { '[' };
                match open.pop() {
                    Some((o, _)) if o == wanted => {}
                    Some((o, line)) => {
                        return Err(Problem {
                            line: *token_line,
                            message: format!("this `{c}` closes the `{o}` of line {}", line + 1),
                        });
                    }
                    None => {
                        return Err(Problem { line: *token_line, message: format!("a `{c}` with nothing to close") });
                    }
                }
                new_line(&mut out, open.len(), false);
                out.push(*c);
                break_due = false;
                space_due = false;
            }
            Token::Comma => {
                // After a `// comment`, the comma can't go on its line.
                if follows_line_comment {
                    new_line(&mut out, open.len(), false);
                }
                out.push(',');
                break_due = true;
                space_due = false;
            }
            Token::Colon => {
                out.push(':');
                space_due = true;
            }
            Token::Comment(comment, line) => {
                // On the line of what came before: stays there.
                if *breaks == 0 && !out.is_empty() {
                    out.push(' ');
                } else {
                    new_line(&mut out, open.len(), blank);
                }
                out.push_str(comment);
                if *line {
                    break_due = true;
                }
                space_due = false;
            }
            Token::Open(_) | Token::Value(_) => {
                if break_due {
                    new_line(&mut out, open.len(), blank);
                } else if space_due {
                    out.push(' ');
                }
                break_due = false;
                space_due = false;
                match token {
                    Token::Open(c) => {
                        out.push(*c);
                        let close = if *c == '{' { '}' } else { ']' };
                        // Empty: `{}` and `[]` stay on their line.
                        if matches!(tokens.get(n + 1), Some((Token::Close(x), _, _)) if *x == close) {
                            out.push(close);
                            skip_close = true;
                        } else {
                            open.push((*c, *token_line));
                            break_due = true;
                        }
                    }
                    Token::Value(value) => out.push_str(value),
                    _ => {}
                }
            }
        }
    }
    if let Some((o, line)) = open.pop() {
        return Err(Problem { line, message: format!("this `{o}` is never closed") });
    }
    while out.ends_with(' ') {
        out.pop();
    }
    if text.ends_with('\n') {
        out.push_str(newline);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two(text: &str) -> String {
        format(text, "  ", "\n").unwrap()
    }

    #[test]
    fn json_is_laid_out() {
        assert_eq!(
            two(r#"{"name":"null","tags":["a","b"],"n":1.50e3,"none":{},"list":[ ],"ok":true}"#),
            "{\n  \"name\": \"null\",\n  \"tags\": [\n    \"a\",\n    \"b\"\n  ],\n  \"n\": 1.50e3,\n  \"none\": {},\n  \"list\": [],\n  \"ok\": true\n}"
        );
        // Already laid out: the same.
        let done = "{\n  \"a\": [\n    1,\n    2\n  ]\n}\n";
        assert_eq!(two(done), done);
        assert_eq!(two("[]"), "[]");
        assert_eq!(two("  42 \n"), "42\n");
    }

    #[test]
    fn comments_and_blank_lines_stay() {
        let text = "// settings\n{\n\"compilerOptions\": { // for the build\n\"strict\": true, /* yes */\n\n\n\"target\": \"es2022\"\n}\n}\n";
        assert_eq!(
            two(text),
            "// settings\n{\n  \"compilerOptions\": { // for the build\n    \"strict\": true, /* yes */\n\n    \"target\": \"es2022\"\n  }\n}\n"
        );
    }

    #[test]
    fn a_comma_after_a_line_comment_goes_below_it() {
        assert_eq!(two("[1 // one\n, 2]"), "[\n  1 // one\n  ,\n  2\n]");
    }

    #[test]
    fn the_file_s_indent_and_line_breaks() {
        assert_eq!(format("{\"a\":[1]}", "\t", "\r\n").unwrap(), "{\r\n\t\"a\": [\r\n\t\t1\r\n\t]\r\n}");
    }

    #[test]
    fn strings_keep_what_they_hold() {
        assert_eq!(two(r#"{"a{":"b,\"c: [d]","é":'x'}"#), "{\n  \"a{\": \"b,\\\"c: [d]\",\n  \"é\": 'x'\n}");
    }

    #[test]
    fn broken_json_says_where() {
        let problem = |text: &str| format(text, "  ", "\n").unwrap_err();
        assert_eq!(
            problem("{\n  \"a\": [1, 2}\n"),
            Problem { line: 1, message: "this `}` closes the `[` of line 2".into() }
        );
        assert_eq!(problem("{\n\"a\": 1\n").line, 0);
        assert_eq!(problem("]").message, "a `]` with nothing to close");
        assert_eq!(problem("{\"a\n}").message, "a string that never ends");
        assert_eq!(problem("/* x").message, "a comment that never ends");
    }

    /// `cargo test timing_json_format -- --ignored --nocapture`: a 5 MB lock file.
    #[test]
    #[ignore]
    fn timing_json_format() {
        let one = r#"{"name":"left-pad","version":"1.3.0","resolved":"https://registry.example/left-pad.tgz","deps":{"a":"^1","b":["x","y"]}}"#;
        let text = format!("[{}]", vec![one; 45_000].join(","));
        let start = std::time::Instant::now();
        let out = format(&text, "  ", "\n").unwrap();
        eprintln!("{} MB in {:?}", text.len() / 1_000_000, start.elapsed());
        assert!(out.len() > text.len());
    }
}
