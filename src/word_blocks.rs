//! Blocks opened and closed by words rather than braces: Ruby's `def … end`, Lua's
//! `then … end`, the shell's `do … done`, and YAML's `key:`. Enter after a line that opens
//! one goes in a level; `end`, `fi` or `else` typed first on a line goes back to the
//! indentation of what it closes.

/// The words of a language's blocks.
struct Words {
    /// Open a block wherever they are on the line.
    openers: &'static [&'static str],
    /// Open a block only first on the line, or after `=` (Ruby's `x = if …`): elsewhere
    /// they're modifiers (`return if done`).
    leading: &'static [&'static str],
    closers: &'static [&'static str],
    /// End one part of a block and start the next (`else`, `when`).
    middles: &'static [&'static str],
    comment: &'static str,
    /// Words count anywhere on the line; in the shell, only as commands (first, or after
    /// `;`): `echo done` doesn't close a loop.
    anywhere: bool,
}

const RUBY: Words = Words {
    openers: &["def", "class", "module", "begin", "case", "do"],
    leading: &["if", "unless", "while", "until", "for"],
    closers: &["end"],
    middles: &["else", "elsif", "when", "rescue", "ensure"],
    comment: "#",
    anywhere: true,
};

const LUA: Words = Words {
    openers: &["function", "then", "do", "repeat"],
    leading: &[],
    closers: &["end", "until"],
    middles: &["else", "elseif"],
    comment: "--",
    anywhere: true,
};

const SHELL: Words = Words {
    openers: &["then", "do", "case"],
    leading: &[],
    closers: &["fi", "done", "esac"],
    middles: &["else", "elif"],
    comment: "#",
    anywhere: false,
};

fn words(language: &str) -> Option<&'static Words> {
    match language {
        "Ruby" => Some(&RUBY),
        "Lua" => Some(&LUA),
        "Shell" => Some(&SHELL),
        _ => None,
    }
}

/// Whether the language has blocks of words, so typing a word may move its line.
pub fn has_words(language: &str) -> bool {
    words(language).is_some()
}

/// The words of a line, outside strings and its comment, each with whether it follows a
/// `.` (a method: `self.class`, `x.end`) and what came before it.
fn line_words<'a>(line: &'a str, comment: &str) -> Vec<(&'a str, bool, char)> {
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut last = ' ';
    let mut word_start: Option<usize> = None;
    let mut before = ' ';
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut chars = line.char_indices().peekable();
    let mut last_is_space = true;
    while let Some((i, c)) = chars.next() {
        if let Some(q) = quote {
            if c == '\\' {
                chars.next();
            } else if c == q {
                quote = None;
            }
            last = c;
            continue;
        }
        if is_word(c) {
            if word_start.is_none() {
                word_start = Some(i);
                before = last;
            }
        } else {
            if let Some(s) = word_start.take() {
                out.push((&line[s..i], not_a_keyword(line, s, i, before), before));
            }
            // `#` starts a comment after a space only (`$#`, `${#a}` are the shell's).
            if line[i..].starts_with(comment) && (comment != "#" || i == 0 || last_is_space) {
                return out;
            }
            if matches!(c, '"' | '\'' | '`') {
                quote = Some(c);
            }
        }
        if !c.is_whitespace() {
            last = c;
        }
        last_is_space = c.is_whitespace();
    }
    if let Some(s) = word_start {
        out.push((&line[s..], not_a_keyword(line, s, line.len(), before), before));
    }
    out
}

/// The word at `start..end` is a name, not a keyword: a method (`self.class`), a symbol
/// (`:end`) or a hash key (`class: "nav"`).
fn not_a_keyword(line: &str, start: usize, end: usize, before: char) -> bool {
    let after = &line[end..];
    before == '.' || line[..start].ends_with(':') || (after.starts_with(':') && !after.starts_with("::"))
}

/// Ruby's one-line method, `def name = value`: it has no `end`.
fn endless_def(after_def: &str) -> bool {
    let b = after_def.as_bytes();
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'=' if depth == 0 => {
                let spaced = i > 0 && matches!(b[i - 1], b' ' | b'\t' | b')');
                let alone = !matches!(b.get(i + 1), Some(b'=' | b'~' | b'>'));
                if spaced && alone {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// How many blocks a line leaves open (less than zero: closes ones opened before), not
/// counting a part word it starts with.
fn net(words: &Words, line: &str) -> i32 {
    let all = line_words(line, words.comment);
    let mut net = 0;
    let mut loops = false;
    // `elseif x then`: the `then` goes with the part word, which neither opens nor closes.
    let part = all.first().is_some_and(|(w, _, _)| words.middles.contains(w));
    for (n, (word, method, before)) in all.iter().enumerate() {
        if part && *word == "then" {
            continue;
        }
        let command = n == 0 || matches!(before, ';' | '&' | '|');
        if *method || (n == 0 && words.middles.contains(word)) || !(words.anywhere || command) {
            continue;
        }
        if words.leading.contains(word) && (n == 0 || *before == '=') {
            net += 1;
            loops |= matches!(*word, "while" | "until" | "for");
        } else if words.openers.contains(word) {
            let at = word.as_ptr() as usize - line.as_ptr() as usize + word.len();
            // Ruby's `while x do`: one block, not two.
            if !(*word == "do" && loops) && !(*word == "def" && endless_def(&line[at..])) {
                net += 1;
            }
        } else if words.closers.contains(word) {
            net -= 1;
        }
    }
    net
}

fn first_word(line: &str) -> &str {
    let t = line.trim_start();
    &t[..t.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(t.len())]
}

/// Whether the line before the caret opens a block, so the next line goes in a level.
pub fn opens(language: &str, before: &str) -> bool {
    let before = before.trim_end();
    if language == "YAML" {
        // `key:`, `- key:`, and a block of text after `key: |` or `key: >`.
        let text = before.split(" #").next().unwrap_or("").trim_end();
        let text = text.trim_end_matches(['-', '+']);
        return !text.trim_start().starts_with('#')
            && (text.ends_with(':') || text.ends_with(": |") || text.ends_with(": >"));
    }
    let Some(words) = words(language) else { return false };
    // Ruby's `each { |x|`.
    if language == "Ruby" && before.ends_with('|') {
        let body = before.trim_end_matches('|');
        if let Some(bar) = body.rfind('|')
            && body[..bar].trim_end().ends_with('{')
        {
            return true;
        }
    }
    let middle = words.middles.contains(&first_word(before));
    net(words, before) > 0 || (middle && net(words, before) >= 0)
}

/// YAML: after a list item that's a map (`- name: web`), the item's next key lines up with
/// its first: the indentation for the next line.
pub fn yaml_item_indent(before: &str) -> Option<String> {
    let item = before.trim_start();
    let leading = &before[..before.len() - item.len()];
    let key = item.strip_prefix('-')?.trim_start_matches(' ');
    let dash = item.len() - key.len();
    let colon = key.find(':')?;
    let plain = !key[..colon].is_empty() && !key[..colon].contains(['"', '\'', '{', '[', '#']);
    let then = &key[colon + 1..];
    (dash > 1 && plain && (then.is_empty() || then.starts_with(' '))).then(|| format!("{leading}{}", " ".repeat(dash)))
}

/// `line` is just a closing or part word (`end`, `else`): the indentation of what it
/// belongs to, the nearest line above that opens a block not closed since. `lines` are the
/// lines above, nearest first.
pub fn closer_indent<'a>(language: &str, line: &str, lines: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let words = words(language)?;
    let word = first_word(line);
    if !(words.closers.contains(&word) || words.middles.contains(&word)) {
        return None;
    }
    let mut depth = 0;
    for above in lines.filter(|l| !l.trim().is_empty()).take(2000) {
        let indent = &above[..above.len() - above.trim_start().len()];
        let middle = words.middles.contains(&first_word(above));
        if middle && depth == 0 {
            return Some(indent);
        }
        depth -= net(words, above);
        if depth < 0 {
            return Some(indent);
        }
    }
    None
}

/// Whether `word` closes a block or starts its next part (`end`, `else`), so typing it may
/// move its line.
pub fn is_block_word(language: &str, word: &str) -> bool {
    words(language).is_some_and(|w| w.closers.contains(&word) || w.middles.contains(&word))
}

/// `word` is a block word with something typed after it (`endpoint`, `file`): the line
/// was moved for the word and shouldn't have been.
pub fn grew_from_word(language: &str, word: &str) -> bool {
    let Some(words) = words(language) else { return false };
    let all = || words.closers.iter().chain(words.middles);
    let is_block_word = |w: &str| all().any(|b| *b == w);
    // On the way to a longer one (`elsei` to `elseif`): not grown out of it.
    if all().any(|b| b.len() > word.len() && b.starts_with(word)) {
        return false;
    }
    let mut cut = word.char_indices().map(|(i, _)| i);
    let last = cut.next_back();
    !is_block_word(word) && last.is_some_and(|i| is_block_word(&word[..i]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_that_open_a_block() {
        for (language, line) in [
            ("Ruby", "def greet(name)"),
            ("Ruby", "class Person < Base"),
            ("Ruby", "  items.each do |item|"),
            ("Ruby", "  items.each { |item|"),
            ("Ruby", "if ready? # a comment"),
            ("Ruby", "result = case kind"),
            ("Ruby", "else"),
            ("Ruby", "when :big"),
            ("Ruby", "while x do"),
            ("Ruby", "def ==(other)"),
            ("Ruby", "def initialize(a = 1, b = {})"),
            ("Ruby", "def name=(value)"),
            ("Ruby", "class Foo::Bar"),
            ("Lua", "function M.setup(opts)"),
            ("Lua", "local f = function(x)"),
            ("Lua", "if a then"),
            ("Lua", "for i = 1, 10 do"),
            ("Lua", "elseif b then"),
            ("Lua", "repeat"),
            ("Shell", "if [ -f x ]; then"),
            ("Shell", "for f in *; do"),
            ("Shell", "case \"$1\" in"),
            ("Shell", "else"),
            ("YAML", "services:"),
            ("YAML", "  - run:"),
            ("YAML", "script: |"),
            ("YAML", "text: >-"),
            ("YAML", "key: # note"),
        ] {
            assert!(opens(language, line), "{language}: {line}");
        }
    }

    #[test]
    fn lines_that_dont() {
        for (language, line) in [
            ("Ruby", "return if done"),
            ("Ruby", "def name = @name.upcase; end"),
            ("Ruby", "puts self.class"),
            ("Ruby", "x = \"do\""),
            ("Ruby", "items.each do |i| puts i end"),
            ("Ruby", "# def commented"),
            ("Ruby", "  link_to \"Home\", root_path, class: \"nav\""),
            ("Ruby", "x = { if: 1, do: 2 }"),
            ("Ruby", "send(:class)"),
            ("Ruby", "def full = \"#{first} #{last}\""),
            ("Ruby", "def area() = width * height"),
            ("Lua", "local f = function() return 1 end"),
            ("Lua", "if a then return end"),
            ("Lua", "x = 1 -- then"),
            ("Shell", "if [ -f x ]; then echo; fi"),
            ("Shell", "echo done"),
            ("Shell", "echo ${#items[@]} # then"),
            ("YAML", "name: web"),
            ("YAML", "  - name: web"),
            ("YAML", "# key:"),
            ("YAML", "url: \"http://x\""),
            ("Python", "def f(x)"),
        ] {
            assert!(!opens(language, line), "{language}: {line}");
        }
    }

    #[test]
    fn a_closing_word_finds_its_opener() {
        let above = |text: &'static str| text.lines().rev().collect::<Vec<_>>().into_iter();
        let ruby = "class A\n  def f\n    if x\n      1\n    end\n    2\n";
        assert_eq!(closer_indent("Ruby", "    end", above(ruby)), Some("  "));
        assert_eq!(closer_indent("Ruby", "      else", above("def f\n  if x\n    1\n")), Some("  "));
        assert_eq!(closer_indent("Ruby", "  end", above("def f\n  if x\n    1\n  else\n    2\n")), Some("  "));
        assert_eq!(closer_indent("Ruby", "  when 2", above("case k\nwhen 1\n  a\n")), Some(""));
        assert_eq!(closer_indent("Lua", "  end", above("function f()\n  return 1\n")), Some(""));
        assert_eq!(closer_indent("Lua", "    elseif", above("  if a then\n    x()\n")), Some("  "));
        assert_eq!(closer_indent("Shell", "  fi", above("if a; then\n  for x in y; do\n    z\n  done\n")), Some(""));
        // Not a closing word, or nothing open above.
        assert_eq!(closer_indent("Ruby", "  ending = 1", above("def f\n")), None);
        assert_eq!(closer_indent("Ruby", "end", above("x = 1\n")), None);
        assert_eq!(closer_indent("Python", "else", above("if x:\n")), None);
    }

    #[test]
    fn a_yaml_item_lines_up_with_its_key() {
        assert_eq!(yaml_item_indent("  - name: web").as_deref(), Some("    "));
        assert_eq!(yaml_item_indent("-   run:").as_deref(), Some("    "));
        assert_eq!(yaml_item_indent("  - plain item"), None);
        assert_eq!(yaml_item_indent("  - \"a: b\""), None);
        assert_eq!(yaml_item_indent("  - http://x"), None);
        assert_eq!(yaml_item_indent("key: value"), None);
    }

    #[test]
    fn a_word_that_grew() {
        assert!(grew_from_word("Ruby", "endp"));
        assert!(grew_from_word("Shell", "fil"));
        assert!(!grew_from_word("Lua", "elsei"), "on the way to `elseif`");
        assert!(!grew_from_word("Lua", "elseif"));
        assert!(!grew_from_word("Ruby", "end"));
        assert!(!grew_from_word("Ruby", "endpoint"), "only right after the word");
    }
}
