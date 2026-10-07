//! Markdown as it reads: a small parser for what READMEs and notes use (headings,
//! emphasis, code, links and images, lists and task lists, quotes, tables, rules), and
//! its drawing for the editor's preview (⌘⇧V).

use crate::fonts::CodeFont;
use crate::theme::Syntax;
use crate::theme::Theme;
use gpui::{
    AnyElement, App, FontStyle, FontWeight, HighlightStyle, InteractiveText, SharedString, StrikethroughStyle,
    StyledText, UnderlineStyle, Window, div, img, prelude::*, px,
};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    /// A fenced (or indented) block: its language, text, and colours.
    Code {
        language: String,
        text: String,
        colours: Vec<(Range<usize>, Syntax)>,
    },
    Quote(Vec<Block>),
    /// `start` is the first number of an ordered list.
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Table {
        head: Vec<Vec<Inline>>,
        align: Vec<Align>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
    /// A footnote's text (`[^label]: …`). Gathered at the end once the document is read.
    Note(String, Vec<Inline>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// A task list's box: ticked or not.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strike(Vec<Inline>),
    Link {
        text: Vec<Inline>,
        url: String,
    },
    Image {
        alt: String,
        url: String,
    },
    Break,
    /// A footnote reference (`[^label]`): its number once the document is read.
    NoteRef(String),
}

// ---------- blocks ----------

#[cfg(test)]
pub fn parse(source: &str) -> Vec<Block> {
    parse_located(source).into_iter().map(|(_, b)| b).collect()
}

/// The text's lines as the editor counts them: a line ends at "\n", "\r\n" or a lone
/// "\r" (old Mac files), so line numbers here are the editor's.
fn buffer_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                i += if bytes.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// The document's blocks, each with the source line it starts on.
pub fn parse_located(source: &str) -> Vec<(usize, Block)> {
    let lines: Vec<&str> = buffer_lines(source);
    // Front matter (--- … --- at the very top) is the file's settings, not its text.
    let mut start = 0;
    if lines.first().is_some_and(|l| l.trim() == "---")
        && let Some(end) = lines.iter().skip(1).position(|l| l.trim() == "---")
    {
        start = end + 2;
    }
    let start = start.min(lines.len());
    let (blocks, starts) = parse_lines_at(&lines[start..]);
    with_footnotes(starts.into_iter().map(|line| line + start).zip(blocks).collect())
}

/// Footnotes as they read: each reference a small number (in the order first referred
/// to), the notes themselves gathered at the end, under a line, numbered alike. A
/// reference to no note stays as written.
fn with_footnotes(blocks: Vec<(usize, Block)>) -> Vec<(usize, Block)> {
    let mut notes: Vec<(usize, String, Vec<Inline>)> = Vec::new();
    let mut kept = Vec::new();
    for (line, block) in blocks {
        match block {
            Block::Note(label, content) => notes.push((line, label, content)),
            block => kept.push((line, block)),
        }
    }
    if notes.is_empty() && !kept.iter().any(|(_, b)| has_note_ref(b)) {
        return kept;
    }
    // Numbered by first reference; notes no one refers to come after.
    let mut order: Vec<String> = Vec::new();
    for (_, block) in &kept {
        note_refs(block, &mut order);
    }
    order.retain(|label| notes.iter().any(|(_, l, _)| l == label));
    for (_, label, _) in &notes {
        if !order.contains(label) {
            order.push(label.clone());
        }
    }
    let number = |label: &str| order.iter().position(|l| l == label).map(|i| i + 1);
    for (_, block) in &mut kept {
        number_refs(block, &number);
    }
    if !notes.is_empty() {
        let first = notes.iter().map(|(line, ..)| *line).min().unwrap_or(0);
        notes.sort_by_key(|(_, label, _)| number(label));
        let items = notes
            .into_iter()
            .map(|(_, _, content)| Item { task: None, blocks: vec![Block::Paragraph(content)] })
            .collect();
        kept.push((first, Block::Rule));
        kept.push((first, Block::List { start: Some(1), items }));
    }
    kept
}

/// The inlines a block holds, at any depth.
fn block_inlines(block: &mut Block, each: &mut dyn FnMut(&mut Vec<Inline>)) {
    match block {
        Block::Heading(_, content) | Block::Paragraph(content) | Block::Note(_, content) => each(content),
        Block::Quote(blocks) => blocks.iter_mut().for_each(|b| block_inlines(b, each)),
        Block::List { items, .. } => {
            items.iter_mut().flat_map(|i| i.blocks.iter_mut()).for_each(|b| block_inlines(b, each))
        }
        Block::Table { head, rows, .. } => {
            head.iter_mut().chain(rows.iter_mut().flatten()).for_each(|cell| each(cell));
        }
        Block::Code { .. } | Block::Rule => {}
    }
}

/// Each inline of `content`, at any depth (inside emphasis, links...).
fn walk_inlines(content: &mut [Inline], each: &mut dyn FnMut(&mut Inline)) {
    for inline in content {
        if let Inline::Strong(inner)
        | Inline::Emphasis(inner)
        | Inline::Strike(inner)
        | Inline::Link { text: inner, .. } = inline
        {
            walk_inlines(inner, each);
        }
        each(inline);
    }
}

fn has_note_ref(block: &Block) -> bool {
    let mut found = Vec::new();
    note_refs(block, &mut found);
    !found.is_empty()
}

/// The labels referred to in `block`, added to `order` the first time each is seen.
fn note_refs(block: &Block, order: &mut Vec<String>) {
    let mut block = block.clone();
    block_inlines(&mut block, &mut |content| {
        walk_inlines(content, &mut |inline| {
            if let Inline::NoteRef(label) = inline
                && !order.contains(label)
            {
                order.push(label.clone());
            }
        })
    });
}

/// References become their note's number, small and raised (¹ ²); one with no note stays
/// as written.
fn number_refs(block: &mut Block, number: &dyn Fn(&str) -> Option<usize>) {
    block_inlines(block, &mut |content| {
        walk_inlines(content, &mut |inline| {
            if let Inline::NoteRef(label) = inline {
                *inline = Inline::Text(match number(label) {
                    Some(n) => superscript(n),
                    None => format!("[^{label}]"),
                });
            }
        })
    });
}

fn superscript(n: usize) -> String {
    n.to_string().chars().map(|d| "⁰¹²³⁴⁵⁶⁷⁸⁹".chars().nth(d.to_digit(10).unwrap_or(0) as usize).unwrap_or(d)).collect()
}

fn parse_lines(lines: &[&str]) -> Vec<Block> {
    parse_lines_at(lines).0
}

/// The blocks of `lines`, and the line each starts on.
fn parse_lines_at(lines: &[&str]) -> (Vec<Block>, Vec<usize>) {
    let mut blocks = Vec::new();
    let mut starts = Vec::new();
    let mut begin = 0;
    let mut i = 0;
    while i < lines.len() {
        // The block just made started where the last turn did.
        starts.resize(blocks.len(), begin);
        begin = i;
        let line = lines[i];
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        // An HTML comment: left out, however many lines it takes.
        if trimmed.starts_with("<!--") {
            while i < lines.len() && !lines[i].contains("-->") {
                i += 1;
            }
            i += 1;
            continue;
        }
        if let Some(fence) = fence_of(trimmed) {
            let language = trimmed[fence.len()..].split_whitespace().next().unwrap_or("").to_string();
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(fence) {
                code.push(lines[i].get(indent.min(lines[i].len() - lines[i].trim_start().len())..).unwrap_or(""));
                i += 1;
            }
            i += 1;
            blocks.push(code_block(language, code.join("\n")));
            continue;
        }
        if indent >= 4 {
            let mut code = Vec::new();
            while i < lines.len() && (lines[i].trim().is_empty() || lines[i].starts_with("    ")) {
                code.push(lines[i].get(4..).unwrap_or(""));
                i += 1;
            }
            while code.last().is_some_and(|l| l.trim().is_empty()) {
                code.pop();
            }
            blocks.push(code_block(String::new(), code.join("\n")));
            continue;
        }
        // A footnote: `[^label]: text`, and the lines after it up to a blank one.
        if let Some((label, first)) = note_definition(trimmed) {
            let mut text = vec![first.trim()];
            i += 1;
            while i < lines.len() && !lines[i].trim().is_empty() && note_definition(lines[i].trim_start()).is_none() {
                text.push(lines[i].trim());
                i += 1;
            }
            blocks.push(Block::Note(label, inlines(&text.join(" "))));
            continue;
        }
        if let Some((level, text)) = heading(trimmed) {
            blocks.push(Block::Heading(level, inlines(text)));
            i += 1;
            continue;
        }
        if is_rule(trimmed) {
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }
        if trimmed.starts_with('>') {
            let mut quoted = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let rest = &lines[i].trim_start()[1..];
                quoted.push(rest.strip_prefix(' ').unwrap_or(rest));
                i += 1;
            }
            blocks.push(Block::Quote(parse_lines(&quoted)));
            continue;
        }
        if let Some(marker) = list_marker(line) {
            let (block, next) = list(lines, i, marker);
            blocks.push(block);
            i = next;
            continue;
        }
        if line.contains('|') && lines.get(i + 1).is_some_and(|l| is_table_rule(l)) {
            let head = cells(line);
            let align = cells_text(lines[i + 1]).iter().map(|c| align_of(c)).collect();
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(cells(lines[i]));
                i += 1;
            }
            blocks.push(Block::Table { head, align, rows });
            continue;
        }
        // A paragraph: lines up to a blank one or another block. A line of === or ---
        // under it makes it a heading instead.
        let mut text: Vec<&str> = Vec::new();
        while i < lines.len() {
            let l = lines[i];
            let t = l.trim_start();
            if t.is_empty() {
                break;
            }
            if !text.is_empty() {
                if t.chars().all(|c| c == '=') {
                    blocks.push(Block::Heading(1, inlines(&text.join("\n"))));
                    text.clear();
                    i += 1;
                    break;
                }
                if t.chars().all(|c| c == '-') && t.len() >= 2 {
                    blocks.push(Block::Heading(2, inlines(&text.join("\n"))));
                    text.clear();
                    i += 1;
                    break;
                }
                if heading(t).is_some() || fence_of(t).is_some() || t.starts_with('>') || list_marker(l).is_some() {
                    break;
                }
            }
            text.push(l.trim());
            i += 1;
        }
        if !text.is_empty() {
            // Two spaces (or a backslash) at a line's end break the line; otherwise lines join.
            let joined =
                text.iter()
                    .enumerate()
                    .map(|(n, l)| {
                        if n + 1 < text.len() && lines_break(lines, l) { format!("{l}\u{2028}") } else { l.to_string() }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
            blocks.push(Block::Paragraph(inlines(&joined)));
        }
    }
    starts.resize(blocks.len(), begin);
    (blocks, starts)
}

/// `[^label]: text` → the label and the text.
fn note_definition(trimmed: &str) -> Option<(String, &str)> {
    let rest = trimmed.strip_prefix("[^")?;
    let close = rest.find("]:")?;
    let label = &rest[..close];
    (!label.is_empty() && !label.contains(char::is_whitespace)).then(|| (label.to_string(), &rest[close + 2..]))
}

fn lines_break(lines: &[&str], trimmed: &str) -> bool {
    trimmed.ends_with('\\') || lines.iter().any(|l| l.trim() == trimmed && l.ends_with("  "))
}

fn code_block(language: String, text: String) -> Block {
    let colours = colours_of(&language, &text);
    Block::Code { language, text, colours }
}

/// The code's colours, from the language its fence names ("rust", "ts", "py"…).
fn colours_of(language: &str, text: &str) -> Vec<(Range<usize>, Syntax)> {
    let extension = match language.to_lowercase().as_str() {
        "rust" => "rs",
        "python" => "py",
        "javascript" | "js" | "jsx" => "js",
        "typescript" | "ts" => "ts",
        "tsx" => "tsx",
        "bash" | "sh" | "shell" | "zsh" | "console" => "sh",
        "golang" | "go" => "go",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "html" => "html",
        "css" => "css",
        "c" => "c",
        "cpp" | "c++" => "cpp",
        other => return colours_for_extension(other, text),
    };
    colours_for_extension(extension, text)
}

fn colours_for_extension(extension: &str, text: &str) -> Vec<(Range<usize>, Syntax)> {
    if extension.is_empty() || text.len() > 200_000 {
        return Vec::new();
    }
    let Some(language) = crate::languages::for_path(Path::new(&format!("x.{extension}"))) else { return Vec::new() };
    let Some(mut highlighter) = crate::highlight::Highlighter::new(language) else { return Vec::new() };
    let buffer = crate::buffer::Buffer::from_text(text);
    highlighter.sync(&buffer);
    highlighter.spans(buffer.rope(), 0..text.len())
}

fn fence_of(trimmed: &str) -> Option<&'static str> {
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn heading(trimmed: &str) -> Option<(u8, &str)> {
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((level as u8, rest.trim().trim_end_matches('#').trim_end()))
}

fn is_rule(trimmed: &str) -> bool {
    let chars: Vec<char> = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    chars.len() >= 3 && ['-', '*', '_'].iter().any(|&m| chars.iter().all(|&c| c == m))
}

/// A list item's start: its indentation, where its text begins, and the number of an
/// ordered one.
#[derive(Clone, Copy)]
struct Marker {
    indent: usize,
    content: usize,
    number: Option<u64>,
}

fn list_marker(line: &str) -> Option<Marker> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    let (marker_len, number) = if trimmed.starts_with(['-', '*', '+']) {
        (1, None)
    } else {
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 9 || !trimmed[digits..].starts_with(['.', ')']) {
            return None;
        }
        (digits + 1, trimmed[..digits].parse().ok())
    };
    let rest = &trimmed[marker_len..];
    if !(rest.starts_with(' ') || rest.is_empty()) || is_rule(trimmed) {
        return None;
    }
    let spaces = rest.chars().take_while(|&c| c == ' ').count().clamp(1, 4);
    Some(Marker { indent, content: indent + marker_len + spaces, number })
}

/// A list from `lines[start]`: its items, each with what's indented under it.
fn list(lines: &[&str], start: usize, first: Marker) -> (Block, usize) {
    let mut items = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let Some(marker) =
            list_marker(lines[i]).filter(|m| m.indent == first.indent && m.number.is_some() == first.number.is_some())
        else {
            break;
        };
        let mut content = vec![lines[i].get(marker.content..).unwrap_or("").to_string()];
        i += 1;
        // The item goes on while lines are indented under it (or follow lazily, without a gap).
        while i < lines.len() {
            let line = lines[i];
            let indent = line.len() - line.trim_start().len();
            if line.trim().is_empty() {
                let next_inside = lines
                    .get(i + 1)
                    .is_some_and(|n| !n.trim().is_empty() && n.len() - n.trim_start().len() >= marker.content);
                if !next_inside {
                    break;
                }
                content.push(String::new());
            } else if indent >= marker.content {
                content.push(line[marker.content.min(indent)..].to_string());
            } else if list_marker(line).is_some()
                || indent < marker.content && content.last().is_some_and(|l| l.is_empty())
            {
                break;
            } else {
                content.push(line.trim().to_string());
            }
            i += 1;
        }
        let mut task = None;
        if let Some(first) = content.first_mut() {
            for (prefix, done) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
                if let Some(rest) = first.strip_prefix(prefix) {
                    task = Some(done);
                    *first = rest.to_string();
                }
            }
        }
        let refs: Vec<&str> = content.iter().map(String::as_str).collect();
        items.push(Item { task, blocks: parse_lines(&refs) });
        // A blank line between items keeps the list going.
        while i < lines.len()
            && lines[i].trim().is_empty()
            && lines.get(i + 1).and_then(|n| list_marker(n)).is_some_and(|m| m.indent == first.indent)
        {
            i += 1;
        }
    }
    (Block::List { start: first.number, items }, i)
}

fn is_table_rule(line: &str) -> bool {
    // At least one pipe: a lone `---` under a line is a heading's underline, or a rule.
    if !line.contains('|') {
        return false;
    }
    let cells = cells_text(line);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim();
            let inner = c.trim_start_matches(':').trim_end_matches(':');
            !inner.is_empty() && inner.chars().all(|ch| ch == '-')
        })
}

fn cells_text(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    // Split at pipes, but not escaped ones or those in code.
    let mut cells = vec![String::new()];
    let mut in_code = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => cells.last_mut().unwrap().push(chars.next().unwrap()),
            '`' => {
                in_code = !in_code;
                cells.last_mut().unwrap().push(c);
            }
            '|' if !in_code => cells.push(String::new()),
            _ => cells.last_mut().unwrap().push(c),
        }
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

/// A table row's cells as written (escaped pipes kept), trimmed.
fn raw_cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = if line.ends_with('|') && !line.ends_with("\\|") { &line[..line.len() - 1] } else { line };
    let mut cells = vec![String::new()];
    let mut in_code = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                let cell = cells.last_mut().unwrap();
                cell.push(c);
                cell.push(chars.next().unwrap());
            }
            '`' => {
                in_code = !in_code;
                cells.last_mut().unwrap().push(c);
            }
            '|' if !in_code => cells.push(String::new()),
            _ => cells.last_mut().unwrap().push(c),
        }
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

/// The tables of a Markdown file with their columns lined up: each table's line range and
/// its new lines (only those that change). Tables in code blocks are left alone.
pub fn aligned_tables(source: &str) -> Vec<(Range<usize>, Vec<String>)> {
    let lines: Vec<&str> = buffer_lines(source);
    let mut found = Vec::new();
    let mut fence: Option<&str> = None;
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            i += 1;
            continue;
        }
        if let Some(open) = fence_of(trimmed) {
            fence = Some(open);
            i += 1;
            continue;
        }
        if !(lines[i].contains('|') && lines.get(i + 1).is_some_and(|l| is_table_rule(l))) {
            i += 1;
            continue;
        }
        let start = i;
        i += 2;
        while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
            i += 1;
        }
        let new = align_table(&lines[start..i]);
        if new.iter().map(String::as_str).ne(lines[start..i].iter().copied()) {
            found.push((start..i, new));
        }
    }
    found
}

/// ⇥ or ⇧⇥ in a table: its lines (`lines`, in the text) lined up as `new`, and where the
/// caret goes, as a line and char column in `new`.
#[derive(Debug, PartialEq)]
pub struct TableStep {
    pub lines: Range<usize>,
    pub new: Vec<String>,
    pub caret: (usize, usize),
}

fn is_row(line: &str) -> bool {
    line.contains('|') && !line.trim().is_empty()
}

/// The pipes that split `line` into cells, as char columns (not escaped, not in code).
fn pipes(line: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut in_code = false;
    let mut escaped = false;
    for (i, c) in line.chars().enumerate() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '`' => in_code = !in_code,
            '|' if !in_code => found.push(i),
            _ => {}
        }
    }
    found
}

/// Where the caret at `column` of line `at` goes with ⇥ (`forward`) or ⇧⇥ in the table it's
/// in, with the table lined up; a ⇥ past the last cell adds a row. None out of a table.
pub fn table_step(lines: &[&str], at: usize, column: usize, forward: bool) -> Option<TableStep> {
    if !is_row(lines.get(at)?) {
        return None;
    }
    let mut top = at;
    while top > 0 && is_row(lines[top - 1]) {
        top -= 1;
    }
    // The head is the row just above the rule.
    let rule = (top + 1..=(at + 1).min(lines.len().saturating_sub(1))).rev().find(|&r| is_table_rule(lines[r]))?;
    let start = rule - 1;
    if at < start {
        return None;
    }
    let mut end = at + 1;
    while end < lines.len() && is_row(lines[end]) {
        end += 1;
    }
    let mut new = align_table(&lines[start..end]);
    let columns = raw_cells(&new[0]).len();
    let row = at - start;
    let leading = lines[at].trim_start().starts_with('|');
    let cell = (pipes(lines[at]).iter().filter(|&&p| p < column).count())
        .saturating_sub(usize::from(leading))
        .min(columns - 1);
    let (row, cell) = if forward {
        match (row, cell + 1 < columns) {
            (1, _) => (2, 0),
            (_, true) => (row, cell + 1),
            (0, false) => (2, 0),
            (_, false) => (row + 1, 0),
        }
    } else {
        match (row, cell) {
            (0, 0) => (0, 0),
            (1, _) => (0, columns - 1),
            (_, 0) if row == 2 => (0, columns - 1),
            (_, 0) => (row - 1, columns - 1),
            (_, cell) => (row, cell - 1),
        }
    };
    if row >= new.len() {
        let empty = format!("|{}", " |".repeat(columns));
        let mut grown: Vec<&str> = new.iter().map(String::as_str).collect();
        grown.push(&empty);
        new = align_table(&grown);
    }
    let line = &new[row];
    let after = pipes(line)[cell] + 1;
    let text_at = line.chars().skip(after).position(|c| c != ' ').map_or(after + 1, |p| after + p);
    // An empty cell: just inside it.
    let caret = if line.chars().nth(text_at) == Some('|') { after + 1 } else { text_at };
    Some(TableStep { lines: start..end, new, caret: (row, caret) })
}

/// One table's lines (head, rule, rows) with every column as wide as its widest cell.
fn align_table(lines: &[&str]) -> Vec<String> {
    if lines.len() < 2 {
        return lines.iter().map(|l| l.to_string()).collect();
    }
    let indent = &lines[0][..lines[0].len() - lines[0].trim_start().len()];
    let rows: Vec<Vec<String>> = lines.iter().map(|l| raw_cells(l)).collect();
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let aligns: Vec<Align> = (0..columns).map(|c| rows[1].get(c).map_or(Align::Left, |r| align_of(r))).collect();
    let explicit_left: Vec<bool> = (0..columns).map(|c| rows[1].get(c).is_some_and(|r| r.starts_with(':'))).collect();
    let width = |c: usize| {
        rows.iter()
            .enumerate()
            .filter(|(n, _)| *n != 1)
            .filter_map(|(_, r)| r.get(c))
            .map(|t| t.chars().count())
            .max()
            .unwrap_or(0)
            .max(3)
    };
    let widths: Vec<usize> = (0..columns).map(width).collect();
    rows.iter()
        .enumerate()
        .map(|(n, row)| {
            let cells: Vec<String> = (0..columns)
                .map(|c| {
                    let w = widths[c];
                    if n == 1 {
                        return match aligns[c] {
                            Align::Center => format!(":{}:", "-".repeat(w - 2)),
                            Align::Right => format!("{}:", "-".repeat(w - 1)),
                            Align::Left if explicit_left[c] => format!(":{}", "-".repeat(w - 1)),
                            Align::Left => "-".repeat(w),
                        };
                    }
                    let text = row.get(c).map_or("", String::as_str);
                    let room = w - text.chars().count();
                    match aligns[c] {
                        Align::Right => format!("{}{text}", " ".repeat(room)),
                        Align::Center => format!("{}{text}{}", " ".repeat(room / 2), " ".repeat(room - room / 2)),
                        Align::Left => format!("{text}{}", " ".repeat(room)),
                    }
                })
                .collect();
            format!("{indent}| {} |", cells.join(" | "))
        })
        .collect()
}

fn cells(line: &str) -> Vec<Vec<Inline>> {
    cells_text(line).iter().map(|c| inlines(c)).collect()
}

fn align_of(cell: &str) -> Align {
    match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    }
}

// ---------- inlines ----------

pub fn inlines(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    let flush = |plain: &mut String, out: &mut Vec<Inline>| {
        if !plain.is_empty() {
            out.push(Inline::Text(std::mem::take(plain)));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let rest: String = chars[i..].iter().collect();
        match c {
            '\\' if chars.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) => {
                plain.push(chars[i + 1]);
                i += 2;
                continue;
            }
            '\u{2028}' => {
                flush(&mut plain, &mut out);
                out.push(Inline::Break);
                i += 1;
                continue;
            }
            '`' => {
                let run = chars[i..].iter().take_while(|&&c| c == '`').count();
                let fence: String = "`".repeat(run);
                if let Some(end) = find_from(&chars, i + run, &fence) {
                    flush(&mut plain, &mut out);
                    let code: String = chars[i + run..end].iter().collect();
                    out.push(Inline::Code(code.trim().to_string()));
                    i = end + run;
                    continue;
                }
            }
            '[' if rest.starts_with("[^") => {
                let label: String = chars[i + 2..].iter().take_while(|c| **c != ']' && !c.is_whitespace()).collect();
                if !label.is_empty() && chars.get(i + 2 + label.chars().count()) == Some(&']') {
                    flush(&mut plain, &mut out);
                    i += 3 + label.chars().count();
                    out.push(Inline::NoteRef(label));
                    continue;
                }
            }
            '!' if rest.starts_with("![") => {
                if let Some((alt, url, len)) = bracketed(&chars, i + 1) {
                    flush(&mut plain, &mut out);
                    out.push(Inline::Image { alt, url });
                    i += 1 + len;
                    continue;
                }
            }
            '[' => {
                if let Some((label, url, len)) = bracketed(&chars, i) {
                    flush(&mut plain, &mut out);
                    out.push(Inline::Link { text: inlines(&label), url });
                    i += len;
                    continue;
                }
            }
            '<' if rest.starts_with("<http") => {
                if let Some(end) = rest.find('>') {
                    flush(&mut plain, &mut out);
                    let url = rest[1..end].to_string();
                    out.push(Inline::Link { text: vec![Inline::Text(url.clone())], url });
                    i += rest[..=end].chars().count();
                    continue;
                }
            }
            '<' if rest.starts_with("<br") => {
                if let Some(end) = rest.find('>') {
                    flush(&mut plain, &mut out);
                    out.push(Inline::Break);
                    i += rest[..=end].chars().count();
                    continue;
                }
            }
            'h' if (rest.starts_with("https://") || rest.starts_with("http://"))
                && (i == 0 || !chars[i - 1].is_alphanumeric()) =>
            {
                let len = rest.chars().take_while(|c| !c.is_whitespace() && *c != ')' && *c != '>').count();
                let url: String = rest.chars().take(len).collect();
                let url = url.trim_end_matches(['.', ',', ';', ':', '!', '?']).to_string();
                flush(&mut plain, &mut out);
                i += url.chars().count();
                out.push(Inline::Link { text: vec![Inline::Text(url.clone())], url });
                continue;
            }
            // ***both***: strong and emphasised at once.
            '*' | '_'
                if chars.get(i + 1) == Some(&c)
                    && chars.get(i + 2) == Some(&c)
                    && chars.get(i + 3).is_some_and(|n| !n.is_whitespace()) =>
            {
                let marker: String = std::iter::repeat_n(c, 3).collect();
                if let Some(end) = find_from(&chars, i + 3, &marker).filter(|&e| !chars[e - 1].is_whitespace()) {
                    flush(&mut plain, &mut out);
                    let inner: String = chars[i + 3..end].iter().collect();
                    out.push(Inline::Strong(vec![Inline::Emphasis(inlines(&inner))]));
                    i = end + 3;
                    continue;
                }
            }
            '*' | '_' | '~' => {
                let double = chars.get(i + 1) == Some(&c);
                let opens = chars.get(i + if double { 2 } else { 1 }).is_some_and(|n| !n.is_whitespace())
                    // In_snake_case, an underscore is just a letter.
                    && !(c == '_' && i > 0 && chars[i - 1].is_alphanumeric());
                if opens && (c != '~' || double) {
                    let marker: String = std::iter::repeat_n(c, if double { 2 } else { 1 }).collect();
                    if let Some(end) = closing(&chars, i + marker.len(), &marker) {
                        flush(&mut plain, &mut out);
                        let inner: String = chars[i + marker.len()..end].iter().collect();
                        let inner = inlines(&inner);
                        out.push(match (c, double) {
                            ('~', _) => Inline::Strike(inner),
                            (_, true) => Inline::Strong(inner),
                            _ => Inline::Emphasis(inner),
                        });
                        i = end + marker.len();
                        continue;
                    }
                }
            }
            _ => {}
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut plain, &mut out);
    out
}

fn find_from(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    (from..chars.len().saturating_sub(needle.len() - 1)).find(|&i| chars[i..i + needle.len()] == needle[..])
}

/// A closing `marker` after `from`: not after a space, and (for `_`) not inside a word.
fn closing(chars: &[char], from: usize, marker: &str) -> Option<usize> {
    let m: Vec<char> = marker.chars().collect();
    let mut i = from + 1;
    while i + m.len() <= chars.len() {
        if chars[i..i + m.len()] == m[..]
            && !chars[i - 1].is_whitespace()
            && !(m[0] == '_' && chars.get(i + m.len()).is_some_and(|c| c.is_alphanumeric()))
            // `**` doesn't close a `*`.
            && !(m.len() == 1 && (chars.get(i + 1) == Some(&m[0]) || chars[i - 1] == m[0]))
        {
            return Some(i);
        }
        if chars[i] == '`' {
            // Code spans hide what's in them.
            if let Some(end) = find_from(chars, i + 1, "`") {
                i = end;
            }
        }
        i += 1;
    }
    None
}

/// `[label](url)` from `start` (at the `[`): the label, the url, and the chars it takes.
fn bracketed(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let mut depth = 0;
    let mut close = None;
    for (n, &c) in chars.iter().enumerate().skip(start) {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(n);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let label: String = chars[start + 1..close].iter().collect();
    // `(<path with spaces>)`: everything to the `>`, spaces and `)` included.
    if chars.get(close + 2) == Some(&'<') {
        let gt = (close + 3..chars.len()).find(|&n| chars[n] == '>')?;
        let end = (gt + 1..chars.len()).find(|&n| chars[n] == ')')?;
        let url: String = chars[close + 3..gt].iter().collect();
        return Some((label, url, end + 1 - start));
    }
    let end = (close + 2..chars.len()).find(|&n| chars[n] == ')')?;
    let target: String = chars[close + 2..end].iter().collect();
    // `(url "title")`: the title is left out.
    let url = target.split_whitespace().next().unwrap_or("").to_string();
    Some((label, url, end + 1 - start))
}

// ---------- drawing ----------

/// What a link does when clicked: an address for the browser, or a file in the project.
#[derive(Clone)]
pub enum Follow {
    Web(String),
    File(PathBuf),
    /// A heading of this page, by its slug (`#getting-started`).
    Heading(String),
}

/// A heading's anchor as GitHub makes it: lowercase, spaces to dashes, other punctuation
/// dropped ("Getting started!" is `getting-started`).
/// A Markdown link from a file in `dir` to `target`: `![name](path)` for an image, else
/// `[name](path)`, the path relative (`../assets/logo.png`), in `<…>` when it has spaces.
pub fn link_to(dir: &std::path::Path, target: &std::path::Path) -> String {
    use std::path::Component;
    let ups: Vec<Component> = dir.components().collect();
    let downs: Vec<Component> = target.components().collect();
    let shared = ups.iter().zip(&downs).take_while(|(a, b)| a == b).count();
    let parts: Vec<String> = std::iter::repeat_n("..".to_string(), ups.len() - shared)
        .chain(downs[shared..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()))
        .collect();
    let path = parts.join("/");
    let path = if path.contains(' ') { format!("<{path}>") } else { path };
    let name = target.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if crate::preview::is_image(target) {
        format!("![{name}]({path})")
    } else {
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
        format!("[{name}]({path})")
    }
}

pub fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// The plain text of inlines (a heading's words, for its slug).
pub fn plain_text(content: &[Inline]) -> String {
    content
        .iter()
        .map(|inline| match inline {
            Inline::Text(t) | Inline::Code(t) => t.clone(),
            Inline::Strong(i) | Inline::Emphasis(i) | Inline::Strike(i) => plain_text(i),
            Inline::Link { text, .. } => plain_text(text),
            Inline::Image { alt, .. } => alt.clone(),
            Inline::Break => " ".into(),
            Inline::NoteRef(label) => format!("[^{label}]"),
        })
        .collect()
}

pub fn follow(url: &str, base: &Path) -> Option<Follow> {
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
        return Some(Follow::Web(url.to_string()));
    }
    // A link within the page (#section) or to a file next to it.
    if let Some(anchor) = url.strip_prefix('#') {
        return (!anchor.is_empty()).then(|| Follow::Heading(anchor.to_lowercase()));
    }
    let path = url.split('#').next().unwrap_or("");
    let file = base.join(path);
    file.exists().then_some(Follow::File(file))
}

/// Opens what a link points to.
pub type Opener = Rc<dyn Fn(Follow, &mut Window, &mut App)>;
/// What a click on a task's box does, given which box it is.
pub type Ticker = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub struct Style {
    pub theme: Theme,
    pub ui_font: SharedString,
    pub code_font: SharedString,
    /// How the code font joins characters (see `fonts::code_features`).
    pub code_features: gpui::FontFeatures,
    /// Where relative links and images are found from: the file's folder.
    pub base: PathBuf,
    pub open: Opener,
    /// A task's box clicked: which one, counting from the top (see [`task_lines`]).
    pub tick: Option<Ticker>,
    /// Boxes drawn so far, to number them.
    pub tasks: std::cell::Cell<usize>,
}

/// The source lines of the task list's boxes (`- [ ] milk`), top to bottom, as the preview
/// draws them: not in front matter or code, and inside quotes too.
pub fn task_lines(source: &str) -> Vec<usize> {
    let lines: Vec<&str> = buffer_lines(source);
    let mut start = 0;
    if lines.first().is_some_and(|l| l.trim() == "---")
        && let Some(end) = lines.iter().skip(1).position(|l| l.trim() == "---")
    {
        start = end + 2;
    }
    let mut fence: Option<&str> = None;
    let mut found = Vec::new();
    for (n, line) in lines.iter().enumerate().skip(start) {
        let mut text = *line;
        while let Some(rest) = text.trim_start().strip_prefix('>') {
            text = rest.strip_prefix(' ').unwrap_or(rest);
        }
        let trimmed = text.trim_start();
        match fence {
            Some(open) => {
                if trimmed.starts_with(open) {
                    fence = None;
                }
                continue;
            }
            None => {
                if let Some(open) = fence_of(trimmed) {
                    fence = Some(open);
                    continue;
                }
            }
        }
        if let Some(marker) = list_marker(text)
            && ["[ ] ", "[x] ", "[X] "].iter().any(|b| text.get(marker.content..).is_some_and(|t| t.starts_with(b)))
        {
            found.push(n);
        }
    }
    found
}

/// The line with its task's box ticked, or unticked: the box right after the list's
/// marker (in a quote too), never a `[ ]` written in the item's text.
pub fn toggle_task(line: &str) -> Option<String> {
    let mut start = 0;
    let mut text = line;
    while let Some(rest) = text.trim_start().strip_prefix('>') {
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        start += text.len() - rest.len();
        text = rest;
    }
    let at = start + list_marker(text)?.content;
    let rest = line.get(at..)?;
    let mark = if rest.starts_with("[ ] ") {
        "x"
    } else if rest.starts_with("[x] ") || rest.starts_with("[X] ") {
        " "
    } else {
        return None;
    };
    Some(format!("{}[{mark}]{}", &line[..at], &line[at + 3..]))
}

/// Each block drawn, in order (the preview scrolls to them one by one).
pub fn render_blocks(blocks: &[(usize, Block)], style: &Style) -> Vec<AnyElement> {
    let mut counter = 0;
    blocks.iter().map(|(_, b)| render_block(b, style, &mut counter, style.theme.foreground)).collect()
}

/// `color` is the text's: muted in a quote or a done task.
fn render_block(block: &Block, style: &Style, counter: &mut usize, color: gpui::Hsla) -> AnyElement {
    let theme = &style.theme;
    *counter += 1;
    let id = *counter;
    match block {
        Block::Heading(level, text) => {
            let size = match level {
                1 => 26.,
                2 => 21.,
                3 => 17.,
                _ => 15.,
            };
            let heading = div()
                .pt(px(if *level <= 2 { 8. } else { 4. }))
                .text_size(px(size))
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(px(size * 1.3))
                .child(rich(text, style, id, color, FontWeight::SEMIBOLD));
            if *level <= 2 {
                heading.pb(px(6.)).border_b_1().border_color(theme.hairline).into_any_element()
            } else {
                heading.into_any_element()
            }
        }
        Block::Paragraph(text) => {
            // A paragraph that's only an image (or images) shows them.
            if text.iter().all(|i| matches!(i, Inline::Image { .. })) && !text.is_empty() {
                return div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .children(text.iter().filter_map(|i| match i {
                        Inline::Image { alt, url } => Some(image(alt, url, style)),
                        _ => None,
                    }))
                    .into_any_element();
            }
            div().child(rich(text, style, id, color, FontWeight::NORMAL)).into_any_element()
        }
        Block::Code { text, colours, .. } => {
            let mut highlights: Vec<(Range<usize>, HighlightStyle)> = colours
                .iter()
                .map(|(r, s)| (r.clone(), HighlightStyle { color: Some(theme.syntax(*s)), ..Default::default() }))
                .collect();
            highlights
                .retain(|(r, _)| r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
            div()
                .id(("code", id))
                .px(px(14.))
                .py(px(10.))
                .rounded(px(crate::ui::R_CONTROL))
                .bg(theme.raised)
                .overflow_x_scroll()
                .code_font_as(style.code_font.clone(), style.code_features.clone())
                .text_size(px(13.))
                .line_height(px(20.))
                .whitespace_nowrap()
                .child(StyledText::new(text.clone()).with_highlights(highlights))
                .into_any_element()
        }
        Block::Quote(blocks) => div()
            .pl(px(14.))
            .border_l_2()
            .border_color(theme.line_strong)
            .text_color(theme.muted)
            .flex()
            .flex_col()
            .gap(px(10.))
            .children(blocks.iter().map(|b| render_block(b, style, counter, theme.muted)))
            .into_any_element(),
        Block::List { start, items } => div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(items.iter().enumerate().map(|(n, item)| {
                let mark: AnyElement = match (item.task, start) {
                    (Some(done), _) => {
                        let task = style.tasks.get();
                        style.tasks.set(task + 1);
                        let tick = style.tick.clone();
                        div()
                            .id(("task", task))
                            .debug_selector(move || format!("task {task}"))
                            .mt(px(4.))
                            .size(px(13.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(if done { theme.caret } else { theme.muted })
                            .when(done, |d| d.bg(theme.caret.opacity(0.85)))
                            // Clicking it ticks it (or not) in the file.
                            .when_some(tick, |d, tick| {
                                d.cursor_pointer().on_click(move |_, window, cx| tick(task, window, cx))
                            })
                            .into_any_element()
                    }
                    (None, Some(first)) => {
                        div().text_color(theme.muted).child(format!("{}.", first + n as u64)).into_any_element()
                    }
                    (None, None) => div().mt(px(9.)).size(px(5.)).rounded_full().bg(theme.muted).into_any_element(),
                };
                div().flex().gap(px(10.)).child(div().w(px(18.)).flex_none().flex().justify_end().child(mark)).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .when(item.task == Some(true), |d| d.text_color(theme.muted))
                        .children(item.blocks.iter().map(|b| {
                            let color = if item.task == Some(true) { theme.muted } else { color };
                            render_block(b, style, counter, color)
                        })),
                )
            }))
            .into_any_element(),
        Block::Table { head, align, rows } => {
            let cell = |content: &Vec<Inline>, column: usize, header: bool, n: usize| {
                let a = align.get(column).copied().unwrap_or(Align::Left);
                div()
                    .flex_1()
                    .min_w(px(60.))
                    .px(px(10.))
                    .py(px(6.))
                    .flex()
                    .when(a == Align::Center, |d| d.justify_center())
                    .when(a == Align::Right, |d| d.justify_end())
                    .when(header, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .child(rich(
                        content,
                        style,
                        n,
                        color,
                        if header { FontWeight::SEMIBOLD } else { FontWeight::NORMAL },
                    ))
            };
            let row_el = |cells: &Vec<Vec<Inline>>, header: bool, n: usize| {
                div()
                    .flex()
                    .border_b_1()
                    .border_color(theme.hairline)
                    .when(header, |d| d.bg(theme.raised))
                    .children(cells.iter().enumerate().map(|(c, content)| cell(content, c, header, n * 100 + c)))
            };
            div()
                .flex()
                .flex_col()
                .rounded(px(crate::ui::R_CONTROL))
                .border_1()
                .border_color(theme.hairline)
                .overflow_hidden()
                .child(row_el(head, true, id * 1000))
                .children(rows.iter().enumerate().map(|(r, cells)| row_el(cells, false, id * 1000 + r + 1)))
                .into_any_element()
        }
        Block::Rule => div().h(px(1.)).my(px(6.)).bg(theme.hairline).into_any_element(),
        // Gathered at the end as a list before drawing: never drawn where it was written.
        Block::Note(..) => div().into_any_element(),
    }
}

fn image(alt: &str, url: &str, style: &Style) -> AnyElement {
    let theme = &style.theme;
    let local = !url.starts_with("http://") && !url.starts_with("https://");
    let file = style.base.join(url);
    if local && file.is_file() {
        let alt = alt.to_string();
        let faint = theme.faint;
        return img(file)
            .max_w_full()
            .with_fallback(move || div().text_color(faint).child(alt.clone()).into_any_element())
            .into_any_element();
    }
    // Images from the web aren't fetched: their description stands in.
    div()
        .text_color(theme.faint)
        .child(if alt.is_empty() { url.to_string() } else { alt.to_string() })
        .into_any_element()
}

/// Inline text with its styles, links clickable. Built from runs, so code can be in the
/// code font; `color` and `weight` are the text's own.
fn rich(content: &[Inline], style: &Style, id: usize, color: gpui::Hsla, weight: FontWeight) -> AnyElement {
    let mut text = String::new();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut links: Vec<(Range<usize>, String)> = Vec::new();
    let mut code: Vec<Range<usize>> = Vec::new();
    flatten(content, &style.theme, HighlightStyle::default(), &mut text, &mut highlights, &mut links, &mut code);
    let mut cuts: Vec<usize> = vec![0, text.len()];
    cuts.extend(highlights.iter().flat_map(|(r, _)| [r.start, r.end]));
    cuts.extend(code.iter().flat_map(|r| [r.start, r.end]));
    cuts.sort_unstable();
    cuts.dedup();
    let runs: Vec<gpui::TextRun> = cuts
        .windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let look = highlights.iter().find(|(r, _)| r.start <= a && b <= r.end).map(|(_, h)| *h).unwrap_or_default();
            let in_code = code.iter().any(|r| r.start <= a && b <= r.end);
            let family = if in_code { style.code_font.clone() } else { style.ui_font.clone() };
            let mut font = gpui::font(family);
            if in_code {
                font.features = style.code_features.clone();
            }
            font.weight = look.font_weight.unwrap_or(weight);
            font.style = look.font_style.unwrap_or(FontStyle::Normal);
            gpui::TextRun {
                len: b - a,
                font,
                color: look.color.unwrap_or(color),
                background_color: look.background_color,
                underline: look.underline,
                strikethrough: look.strikethrough,
            }
        })
        .collect();
    let styled = StyledText::new(text).with_runs(runs);
    if links.is_empty() {
        return styled.into_any_element();
    }
    let ranges: Vec<Range<usize>> = links.iter().map(|(r, _)| r.clone()).collect();
    let urls: Vec<String> = links.into_iter().map(|(_, u)| u).collect();
    let open = style.open.clone();
    let base = style.base.clone();
    InteractiveText::new(("md-text", id), styled)
        .on_click(ranges, move |ix, window, cx| {
            if let Some(target) = urls.get(ix).and_then(|u| follow(u, &base)) {
                open(target, window, cx);
            }
        })
        .into_any_element()
}

fn flatten(
    content: &[Inline],
    theme: &Theme,
    around: HighlightStyle,
    text: &mut String,
    highlights: &mut Vec<(Range<usize>, HighlightStyle)>,
    links: &mut Vec<(Range<usize>, String)>,
    code: &mut Vec<Range<usize>>,
) {
    for inline in content {
        let start = text.len();
        match inline {
            Inline::Text(t) => {
                text.push_str(t);
                if around != HighlightStyle::default() {
                    highlights.push((start..text.len(), around));
                }
            }
            Inline::Break => text.push('\n'),
            Inline::NoteRef(label) => text.push_str(&format!("[^{label}]")),
            Inline::Code(snippet) => {
                // A little room either side, as the tint would otherwise touch the letters.
                text.push_str(&format!("\u{2009}{snippet}\u{2009}"));
                let style = HighlightStyle { background_color: Some(theme.hairline), ..around };
                highlights.push((start..text.len(), style));
                code.push(start..text.len());
            }
            Inline::Strong(inner) => {
                let style = HighlightStyle { font_weight: Some(FontWeight::SEMIBOLD), ..around };
                flatten(inner, theme, style, text, highlights, links, code);
            }
            Inline::Emphasis(inner) => {
                let style = HighlightStyle { font_style: Some(FontStyle::Italic), ..around };
                flatten(inner, theme, style, text, highlights, links, code);
            }
            Inline::Strike(inner) => {
                let style = HighlightStyle {
                    strikethrough: Some(StrikethroughStyle { thickness: px(1.), color: Some(theme.muted) }),
                    ..around
                };
                flatten(inner, theme, style, text, highlights, links, code);
            }
            Inline::Link { text: label, url } => {
                let style = HighlightStyle {
                    color: Some(theme.caret),
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(theme.caret.opacity(0.5)),
                        wavy: false,
                    }),
                    ..around
                };
                flatten(label, theme, style, text, highlights, links, code);
                links.push((start..text.len(), url.clone()));
            }
            Inline::Image { alt, url } => {
                // In the middle of text, an image is its description, linked.
                text.push_str(if alt.is_empty() { url } else { alt });
                let style = HighlightStyle { color: Some(theme.muted), ..around };
                highlights.push((start..text.len(), style));
                links.push((start..text.len(), url.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Footnotes: references numbered by first use, notes gathered at the end in that
    /// order (after a line), a reference to no note left as written.
    #[test]
    fn footnotes_are_numbered_and_gathered() {
        let source = "Text[^b] and *more*[^a].\n\n[^a]: First, *said*.\n[^b]: Second\ngoes on.\n\nNo note[^none].\n";
        let blocks = parse(source);
        let text = |b: &Block| match b {
            Block::Paragraph(content) => plain_text(content),
            _ => String::new(),
        };
        assert_eq!(text(&blocks[0]), "Text¹ and more².");
        assert_eq!(text(&blocks[1]), "No note[^none].");
        assert_eq!(blocks[2], Block::Rule);
        let Block::List { start: Some(1), items } = &blocks[3] else { panic!("{:?}", blocks[3]) };
        let notes: Vec<String> = items.iter().map(|i| text(&i.blocks[0])).collect();
        assert_eq!(notes, ["Second goes on.", "First, said."]);
        assert_eq!(blocks.len(), 4);
        // No footnotes: nothing added.
        assert_eq!(parse("Just [a link](x).\n").len(), 1);
    }

    #[test]
    fn links_in_angle_brackets_keep_their_spaces() {
        let blocks = parse("![shot](<My Shot.png>) and [doc](<a b).md>)");
        let Block::Paragraph(inlines) = &blocks[0] else { panic!("{blocks:?}") };
        assert!(matches!(&inlines[0], Inline::Image { url, .. } if url == "My Shot.png"), "{inlines:?}");
        assert!(inlines.iter().any(|i| matches!(i, Inline::Link { url, .. } if url == "a b).md")), "{inlines:?}");
    }

    #[test]
    fn lines_are_counted_as_the_editor_counts_them() {
        assert_eq!(buffer_lines("a\nb\r\nc\rd\n"), ["a", "b", "c", "d"]);
        assert_eq!(buffer_lines("x"), ["x"]);
        assert!(buffer_lines("").is_empty());
        // A lone "\r" before a task moves it a line down, as in the editor.
        assert_eq!(task_lines("note\r- [ ] milk\n"), [1]);
    }

    #[test]
    fn tab_goes_from_cell_to_cell() {
        let lines = ["Shop:", "|Item|Qty|", "|-|-|", "|milk|2|"];
        let step = |at: usize, column: usize, forward: bool| table_step(&lines, at, column, forward).unwrap();
        // From "Item": to "Qty", the table lined up.
        let first = step(1, 2, true);
        assert_eq!(first.lines, 1..4);
        assert_eq!(first.new, ["| Item | Qty |", "| ---- | --- |", "| milk | 2   |"]);
        assert_eq!(first.caret, (0, 9));
        // From "Qty": to the next row's first cell, past the rule.
        assert_eq!(step(1, 7, true).caret, (2, 2));
        // From the last cell: a new row.
        let grown = step(3, 7, true);
        assert_eq!(grown.new.len(), 4);
        assert_eq!(grown.new[3], "|      |     |");
        assert_eq!(grown.caret, (3, 2));
        // ⇧⇥ goes back, up past the rule, and stays at the very first cell.
        assert_eq!(step(3, 2, false).caret, (0, 9));
        assert_eq!(step(1, 2, false).caret, (0, 2));
        assert_eq!(table_step(&lines, 0, 0, true), None);
        // A rule line after the table isn't the table's: ⇥ on its last row still works.
        let ruled = ["|h|i|", "|-|-|", "|a|b|", "---"];
        assert_eq!(table_step(&ruled, 2, 1, true).unwrap().lines, 0..3);
        // A heading underlined with `---` isn't a table.
        assert!(aligned_tables("a | b\n---\n").is_empty());
    }

    #[test]
    fn tables_are_lined_up() {
        let doc = "Intro\n\n|Name|Qty|Note|\n|:-:|--:|---|\n|milk|2|`a|b` and \\| pipe|\n|eggs|12|\n\n```\n|a|b|\n|-|-|\n```\n";
        let found = aligned_tables(doc);
        assert_eq!(found.len(), 1, "the table in code is left alone");
        assert_eq!(found[0].0, 2..6);
        let notes = " ".repeat(17);
        assert_eq!(
            found[0].1,
            [
                "| Name | Qty | Note              |".to_string(),
                "| :--: | --: | ----------------- |".to_string(),
                "| milk |   2 | `a|b` and \\| pipe |".to_string(),
                format!("| eggs |  12 | {notes} |"),
            ]
        );
        // Lined up already: nothing to change.
        let lined: String = found[0].1.iter().map(|l| format!("{l}\n")).collect();
        assert!(aligned_tables(&lined).is_empty());
        // In a list item, the indentation stays.
        assert_eq!(aligned_tables("- x\n  |a|b|\n  |-|-|\n")[0].1[0], "  | a   | b   |");
    }

    #[test]
    fn task_boxes_are_found_where_the_preview_draws_them() {
        let doc = "---\ntitle: x\n---\n- [ ] milk\n- [x] eggs\n  - [ ] free range\n\n```\n- [ ] not a task\n```\n> - [X] quoted\n- plain\n";
        assert_eq!(task_lines(doc), [3, 4, 5, 10]);
        // As many as the preview draws.
        let drawn: usize = parse(doc).iter().map(count_tasks).sum();
        assert_eq!(drawn, 4);
        assert_eq!(toggle_task("- [ ] milk").as_deref(), Some("- [x] milk"));
        assert_eq!(toggle_task("  1. [X] eggs").as_deref(), Some("  1. [ ] eggs"));
        assert_eq!(toggle_task("- plain"), None);
        // The box, not a `[ ]` in the text; in a quote too.
        assert_eq!(toggle_task("- [x] support [ ] syntax").as_deref(), Some("- [ ] support [ ] syntax"));
        assert_eq!(toggle_task("> - [ ] quoted").as_deref(), Some("> - [x] quoted"));
    }

    fn count_tasks(block: &Block) -> usize {
        match block {
            Block::List { items, .. } => items
                .iter()
                .map(|i| usize::from(i.task.is_some()) + i.blocks.iter().map(count_tasks).sum::<usize>())
                .sum(),
            Block::Quote(blocks) => blocks.iter().map(count_tasks).sum(),
            _ => 0,
        }
    }

    #[test]
    fn dropped_files_become_relative_links() {
        let dir = std::path::Path::new("/p/docs");
        assert_eq!(link_to(dir, std::path::Path::new("/p/docs/shot.png")), "![shot](shot.png)");
        assert_eq!(link_to(dir, std::path::Path::new("/p/assets/logo.svg")), "![logo](../assets/logo.svg)");
        assert_eq!(link_to(dir, std::path::Path::new("/p/docs/specs/api.pdf")), "[api.pdf](specs/api.pdf)");
        assert_eq!(link_to(dir, std::path::Path::new("/p/docs/My Shot.PNG")), "![My Shot](<My Shot.PNG>)");
    }

    fn text(s: &str) -> Inline {
        Inline::Text(s.into())
    }

    #[test]
    fn reads_inline_styles() {
        assert_eq!(
            inlines("a **bold** and *it* `code` ~~gone~~"),
            vec![
                text("a "),
                Inline::Strong(vec![text("bold")]),
                text(" and "),
                Inline::Emphasis(vec![text("it")]),
                text(" "),
                Inline::Code("code".into()),
                text(" "),
                Inline::Strike(vec![text("gone")]),
            ]
        );
        assert_eq!(inlines("snake_case_name stays"), vec![text("snake_case_name stays")]);
        assert_eq!(inlines("2 * 3 * 4"), vec![text("2 * 3 * 4")]);
        assert_eq!(
            inlines("see [the docs](https://x.dev \"Docs\") or https://y.dev."),
            vec![
                text("see "),
                Inline::Link { text: vec![text("the docs")], url: "https://x.dev".into() },
                text(" or "),
                Inline::Link { text: vec![text("https://y.dev")], url: "https://y.dev".into() },
                text("."),
            ]
        );
        assert_eq!(
            inlines("![logo](assets/logo.png)"),
            vec![Inline::Image { alt: "logo".into(), url: "assets/logo.png".into() }]
        );
        assert_eq!(inlines(r"not \*emphasis\*"), vec![text("not *emphasis*")]);
        assert_eq!(inlines("***both***"), vec![Inline::Strong(vec![Inline::Emphasis(vec![text("both")])])]);
    }

    #[test]
    fn reads_blocks() {
        let doc = "---\ntitle: x\n---\n# Null\n\nSome *text*\nacross lines.\n\n```rust\nfn main() {}\n```\n\n- one\n- [x] done\n  - nested\n\n1. first\n2. second\n\n> quoted\n\n| a | b |\n|:--|--:|\n| 1 | 2 |\n\n---\n<!-- hidden -->\nEnd\n===\n";
        let blocks = parse(doc);
        assert_eq!(blocks[0], Block::Heading(1, vec![text("Null")]));
        assert_eq!(
            blocks[1],
            Block::Paragraph(vec![text("Some "), Inline::Emphasis(vec![text("text")]), text(" across lines.")])
        );
        assert!(
            matches!(&blocks[2], Block::Code { language, text, colours } if language == "rust" && text == "fn main() {}" && !colours.is_empty())
        );
        let Block::List { start: None, items } = &blocks[3] else { panic!("{:?}", blocks[3]) };
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].task, Some(true));
        assert!(matches!(items[1].blocks[1], Block::List { .. }), "{:?}", items[1].blocks);
        assert!(matches!(&blocks[4], Block::List { start: Some(1), items } if items.len() == 2));
        assert_eq!(blocks[5], Block::Quote(vec![Block::Paragraph(vec![text("quoted")])]));
        let Block::Table { head, align, rows } = &blocks[6] else { panic!("{:?}", blocks[6]) };
        assert_eq!((head.len(), align.as_slice(), rows.len()), (2, [Align::Left, Align::Right].as_slice(), 1));
        assert_eq!(blocks[7], Block::Rule);
        assert_eq!(blocks[8], Block::Heading(1, vec![text("End")]));
        assert_eq!(blocks.len(), 9);
        assert_eq!(slug("Getting started!"), "getting-started");
        assert_eq!(slug(&plain_text(&inlines("The `null` *command*"))), "the-null-command");
        // Where each starts in the source, front matter counted.
        let starts: Vec<usize> = parse_located(doc).iter().map(|(line, _)| *line).collect();
        assert_eq!(starts, [3, 5, 8, 12, 16, 19, 21, 25, 27]);
    }
}

/// What Enter carries on to the next line of a Markdown list or quote, from the text
/// before the caret: the next line's start ("- ", "3. ", "- [ ] ", "> "), and whether
/// the item was left empty (then Enter ends the list instead).
pub fn continuation(before_caret: &str) -> Option<(String, bool)> {
    let indent_len = before_caret.len() - before_caret.trim_start_matches([' ', '\t']).len();
    let (indent, mut rest) = before_caret.split_at(indent_len);
    let mut quotes = String::new();
    while let Some(after) = rest.strip_prefix('>') {
        quotes.push('>');
        rest = after;
        if let Some(after) = rest.strip_prefix(' ') {
            quotes.push(' ');
            rest = after;
        }
    }
    let mut next = String::new();
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let marker = if rest.starts_with(['-', '*', '+']) {
        Some((rest[..1].to_string(), 1))
    } else if (1..=9).contains(&digits) && rest[digits..].starts_with(['.', ')']) {
        let number: u64 = rest[..digits].parse().ok()?;
        Some((format!("{}{}", number + 1, &rest[digits..digits + 1]), digits + 1))
    } else {
        None
    };
    if let Some((mark, len)) = marker {
        let spaces = rest[len..].chars().take_while(|&c| c == ' ').count();
        // "-text" isn't an item, and "---" is a rule.
        if spaces == 0 {
            return None;
        }
        next.push_str(&mark);
        next.push_str(&" ".repeat(spaces));
        rest = &rest[len + spaces..];
        for task in ["[ ] ", "[x] ", "[X] "] {
            if let Some(after) = rest.strip_prefix(task) {
                next.push_str("[ ] ");
                rest = after;
            }
        }
    }
    if quotes.is_empty() && next.is_empty() {
        return None;
    }
    Some((format!("{indent}{quotes}{next}"), rest.trim().is_empty()))
}

#[cfg(test)]
mod continuation_tests {
    use super::continuation;

    #[test]
    fn lists_and_quotes_carry_on() {
        assert_eq!(continuation("- milk"), Some(("- ".into(), false)));
        assert_eq!(continuation("  * nested"), Some(("  * ".into(), false)));
        assert_eq!(continuation("9. ninth"), Some(("10. ".into(), false)));
        assert_eq!(continuation("1) first"), Some(("2) ".into(), false)));
        assert_eq!(continuation("- [x] done"), Some(("- [ ] ".into(), false)));
        assert_eq!(continuation("> quoted"), Some(("> ".into(), false)));
        assert_eq!(continuation("> - in a quote"), Some(("> - ".into(), false)));
        // An empty item: Enter ends the list.
        assert_eq!(continuation("- "), Some(("- ".into(), true)));
        assert_eq!(continuation("- [ ] "), Some(("- [ ] ".into(), true)));
        // Not lists.
        assert_eq!(continuation("plain text"), None);
        assert_eq!(continuation("-dash"), None);
        assert_eq!(continuation("---"), None);
        assert_eq!(continuation("2026 was a year"), None);
    }
}
