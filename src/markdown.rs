//! Turns markdown from language servers and AI answers into simple blocks to draw:
//! code blocks, and paragraphs of plain text.

use regex::Regex;
use std::sync::LazyLock;

pub struct Block {
    pub code: bool,
    pub text: String,
}

static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").unwrap());

/// Splits `markdown` at code fences (and `---` rules). `max_lines` caps the total.
pub fn blocks(markdown: &str, max_lines: Option<usize>) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    let mut in_code = false;
    let flush = |lines: &mut Vec<&str>, code: bool, blocks: &mut Vec<Block>| {
        let text = lines.join("\n");
        lines.clear();
        let text = if code { text.trim_end().to_string() } else { clean(&text) };
        if !text.trim().is_empty() {
            blocks.push(Block { code, text });
        }
    };
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            flush(&mut lines, in_code, &mut blocks);
            in_code = !in_code;
        } else if !in_code && line.trim() == "---" {
            flush(&mut lines, false, &mut blocks);
        } else {
            lines.push(line);
        }
    }
    flush(&mut lines, in_code, &mut blocks);

    let Some(mut budget) = max_lines else { return blocks };
    blocks
        .into_iter()
        .filter_map(|mut b| {
            if budget == 0 {
                return None;
            }
            let lines: Vec<&str> = b.text.lines().take(budget).collect();
            budget -= lines.len();
            b.text = lines.join("\n");
            Some(b)
        })
        .collect()
}

/// Readable plain text: no heading marks, links or emphasis.
pub fn clean(text: &str) -> String {
    let text = LINK.replace_all(text, "$1").replace("**", "").replace("__", "").replace('`', "");
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let line = if let Some(rest) = trimmed.strip_prefix('#') {
            rest.trim_start_matches('#').trim().to_string()
        } else if let Some(rest) = trimmed.strip_prefix("* ").or_else(|| trimmed.strip_prefix("- ")) {
            format!("• {rest}")
        } else if let Some(rest) = trimmed.strip_prefix('>') {
            rest.trim().to_string()
        } else {
            line.to_string()
        };
        if line.trim().is_empty() && out.last().is_none_or(|l| l.trim().is_empty()) {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_code_and_text() {
        let b = blocks("Use it:\n```rust\nlet x = 1;\n```\n**Done**.", None);
        let parts: Vec<(bool, &str)> = b.iter().map(|b| (b.code, b.text.as_str())).collect();
        assert_eq!(parts, vec![(false, "Use it:"), (true, "let x = 1;"), (false, "Done.")]);
    }

    #[test]
    fn cleans_headings_lists_and_quotes() {
        assert_eq!(clean("# Title\n\n\n* one\n> note"), "Title\n\n• one\nnote");
    }
}
