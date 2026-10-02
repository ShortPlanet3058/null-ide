//! Which rows things take on screen: lines split by word wrap, and blocks — rows
//! between lines that aren't text, like a line being typed to the AI, lines it
//! removed, or a note. Code fonts are monospaced, so widths are counted in characters.
//!
//! With wrapping off and no blocks every line is one row, and all of this is a no-op.

use crate::buffer::Buffer;
use std::ops::Range;

/// Continuation rows keep the line's indentation, unless that would leave less room than this.
const MIN_ROOM: usize = 20;

/// Rows put between lines: `rows` of them just above `before_line`
/// (or after the last line when it's past the end).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockSpec {
    pub before_line: usize,
    pub rows: usize,
}

/// Which rows each line takes on screen.
pub struct WrapMap {
    /// Characters per row, or None when wrapping is off.
    width: Option<usize>,
    version: u64,
    lines: usize,
    blocks: Vec<BlockSpec>,
    /// Off: one row per line, nothing stored.
    general: bool,
    /// Where each block's rows start, and which block it is, in row order.
    block_rows: Vec<(usize, usize)>,
    /// For each line, the columns where its rows start; the first is always 0.
    starts: Vec<Vec<usize>>,
    /// For each line, how far its continuation rows are indented, in columns.
    indents: Vec<usize>,
    /// The first text row of each line (after any blocks above it), then the total number of rows.
    first_rows: Vec<usize>,
}

/// One row on screen: part (or all) of a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    /// The line's columns shown on this row.
    pub cols: Range<usize>,
    /// Empty columns before the text (the indent of a continuation row).
    pub indent: usize,
    /// Whether this is the line's last row (where its end, and the line break, are).
    pub last: bool,
    /// A block's row instead of text: which block, and which of its rows.
    pub block: Option<(usize, usize)>,
}

impl Default for WrapMap {
    fn default() -> Self {
        Self {
            width: None,
            version: u64::MAX,
            lines: 1,
            blocks: Vec::new(),
            general: false,
            block_rows: Vec::new(),
            starts: Vec::new(),
            indents: Vec::new(),
            first_rows: Vec::new(),
        }
    }
}

impl WrapMap {
    /// Characters per row, or None when wrapping is off.
    pub fn width(&self) -> Option<usize> {
        self.width
    }

    pub fn is_on(&self) -> bool {
        self.width.is_some()
    }

    /// Brings the map up to date with the text, the row width (None to stop wrapping)
    /// and the blocks between lines.
    pub fn update(&mut self, buffer: &Buffer, width: Option<usize>, blocks: &[BlockSpec]) {
        let width = width.map(|w| w.max(MIN_ROOM));
        if width == self.width
            && buffer.version() == self.version
            && buffer.len_lines() == self.lines
            && blocks == self.blocks.as_slice()
        {
            return;
        }
        self.width = width;
        self.version = buffer.version();
        self.lines = buffer.len_lines();
        self.blocks = blocks.to_vec();
        self.starts.clear();
        self.indents.clear();
        self.first_rows.clear();
        self.block_rows.clear();
        self.general = width.is_some() || !blocks.is_empty();
        if !self.general {
            return;
        }
        let mut order: Vec<usize> = (0..blocks.len()).collect();
        order.sort_by_key(|&i| blocks[i].before_line);
        let mut pending = order.into_iter().peekable();
        let mut row = 0;
        let mut texts = buffer.rope().lines();
        for line in 0..self.lines {
            while let Some(&b) = pending.peek().filter(|&&b| blocks[b].before_line <= line) {
                self.block_rows.push((row, b));
                row += blocks[b].rows;
                pending.next();
            }
            // Ropey may not yield the empty line after a final line break.
            let text: String =
                texts.next().map(|l| l.chars().filter(|c| *c != '\n' && *c != '\r').collect()).unwrap_or_default();
            let (starts, indent) = match width {
                Some(width) => wrap_line(&text, width),
                None => (vec![0], 0),
            };
            self.first_rows.push(row);
            row += starts.len();
            self.starts.push(starts);
            self.indents.push(indent);
        }
        for b in pending {
            self.block_rows.push((row, b));
            row += blocks[b].rows;
        }
        self.first_rows.push(row);
    }

    pub fn rows(&self) -> usize {
        if self.general { *self.first_rows.last().unwrap_or(&1) } else { self.lines }
    }

    /// The first text row of `line`. Past the last line, the number of rows.
    pub fn first_row(&self, line: usize) -> usize {
        if self.general { self.first_rows[line.min(self.lines)] } else { line.min(self.lines) }
    }

    /// The rows `line`'s own text takes (not the blocks around it).
    pub fn text_rows(&self, line: usize) -> Range<usize> {
        if !self.general {
            let line = line.min(self.lines);
            return line..line + 1;
        }
        let line = line.min(self.lines.saturating_sub(1));
        let start = self.first_rows[line];
        start..start + self.starts[line].len()
    }

    /// The block shown on `row`, if it isn't text: the block and the row within it.
    pub fn block_at(&self, row: usize) -> Option<(usize, usize)> {
        let i = self.block_rows.partition_point(|&(start, _)| start <= row).checked_sub(1)?;
        let (start, block) = self.block_rows[i];
        (row < start + self.blocks[block].rows).then_some((block, row - start))
    }

    /// The first row of a block.
    pub fn block_row(&self, block: usize) -> Option<usize> {
        self.block_rows.iter().find(|(_, b)| *b == block).map(|(row, _)| *row)
    }

    pub fn line_of_row(&self, row: usize) -> usize {
        if !self.general {
            return row.min(self.lines.saturating_sub(1));
        }
        self.first_rows[..self.lines].partition_point(|&r| r <= row).saturating_sub(1).min(self.lines.saturating_sub(1))
    }

    pub fn row(&self, row: usize, buffer: &Buffer) -> Row {
        if let Some((block, i)) = self.block_at(row) {
            let line = self.blocks[block].before_line.min(self.lines.saturating_sub(1));
            return Row { line, cols: 0..0, indent: 0, last: false, block: Some((block, i)) };
        }
        let line = self.line_of_row(row);
        if !self.general {
            return Row { line, cols: 0..buffer.line_len(line), indent: 0, last: true, block: None };
        }
        let starts = &self.starts[line];
        let i = (row - self.first_rows[line]).min(starts.len() - 1);
        let last = i + 1 == starts.len();
        let end = if last { buffer.line_len(line) } else { starts[i + 1] };
        Row { line, cols: starts[i]..end, indent: if i > 0 { self.indents[line] } else { 0 }, last, block: None }
    }

    /// Where a (line, column) shows: its row, and its column on that row counting the indent.
    pub fn to_display(&self, line: usize, col: usize) -> (usize, usize) {
        if !self.general {
            return (line, col);
        }
        let line = line.min(self.lines - 1);
        let starts = &self.starts[line];
        let i = starts.partition_point(|&s| s <= col).saturating_sub(1);
        let indent = if i > 0 { self.indents[line] } else { 0 };
        (self.first_rows[line] + i, col - starts[i] + indent)
    }

    /// The char offset shown at `col` on `row`. Past the end of a wrapped row, the caret
    /// stays on that row rather than jumping to the start of the next one. On a block's
    /// row, the start of the line below it.
    pub fn to_offset(&self, row: usize, col: usize, buffer: &Buffer) -> usize {
        let row = self.row(row.min(self.rows().saturating_sub(1)), buffer);
        if let Some((block, _)) = row.block {
            let below = self.blocks[block].before_line;
            return if below >= self.lines { buffer.len_chars() } else { buffer.line_to_char(below) };
        }
        let col = row.cols.start + col.saturating_sub(row.indent);
        let max = if row.last { row.cols.end } else { row.cols.end.saturating_sub(1).max(row.cols.start) };
        buffer.offset(row.line, col.min(max))
    }
}

/// Where a line's rows start, and how far its continuation rows are indented.
/// Breaks go after a space when one fits, so words stay whole; a word longer than
/// the row is cut where the row ends.
fn wrap_line(text: &str, width: usize) -> (Vec<usize>, usize) {
    let chars: Vec<char> = text.chars().collect();
    let mut starts = vec![0];
    if chars.len() <= width {
        return (starts, 0);
    }
    let lead = chars.iter().take_while(|c| **c == ' ' || **c == '\t').count();
    let indent = if lead + MIN_ROOM <= width { lead } else { 0 };
    let mut start = 0;
    loop {
        let room = if starts.len() == 1 { width } else { width - indent };
        if chars.len() - start <= room {
            break;
        }
        let limit = start + room;
        let after_space = |i: &usize| chars[*i - 1] == ' ' && chars[*i] != ' ';
        let after_punct = |i: &usize| matches!(chars[*i - 1], ',' | ';' | '(' | '[' | '{' | '.' | '/' | '-');
        let earliest = start + room / 3;
        let brk = (earliest.max(start + 1)..=limit)
            .rev()
            .find(after_space)
            .or_else(|| (earliest.max(start + 1)..=limit).rev().find(after_punct))
            .unwrap_or(limit);
        starts.push(brk);
        start = brk;
    }
    (starts, indent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaks_after_spaces_and_cuts_long_words() {
        assert_eq!(wrap_line("short", 20), (vec![0], 0));
        let text = "aaaa bbbb cccc dddd eeee ffff gggg";
        let (starts, _) = wrap_line(text, 20);
        assert_eq!(starts, vec![0, 20]);
        let (starts, _) = wrap_line(&"x".repeat(45), 20);
        assert_eq!(starts, vec![0, 20, 40]);
    }

    #[test]
    fn continuation_rows_keep_the_indent() {
        let text = format!("    {}", "word ".repeat(10));
        let (starts, indent) = wrap_line(&text, 24);
        assert_eq!(indent, 4);
        assert_eq!(starts[1], 24);
        // The second row has 20 columns of room after its indent.
        assert!(starts[2] - starts[1] <= 20);
    }

    #[test]
    fn maps_rows_and_positions_both_ways() {
        let buffer = Buffer::from_text(&format!("short\n{}\nend\n", "abcd ".repeat(10)));
        let mut map = WrapMap::default();
        map.update(&buffer, Some(20), &[]);
        // Line 1 (50 chars) takes three rows.
        assert_eq!(map.rows(), 1 + 3 + 1 + 1);
        assert_eq!(map.first_row(2), 4);
        assert_eq!(map.line_of_row(3), 1);
        assert_eq!(map.to_display(1, 22), (2, 2));
        assert_eq!(map.to_offset(2, 2, &buffer), buffer.offset(1, 22));
        // Past the end of a wrapped row: stays on that row.
        assert_eq!(map.to_offset(1, 50, &buffer), buffer.offset(1, 19));
        // Past the end of a line's last row: the end of the line.
        assert_eq!(map.to_offset(3, 50, &buffer), buffer.offset(1, 50));

        map.update(&buffer, None, &[]);
        assert_eq!(map.rows(), 4);
        assert_eq!(map.to_display(1, 22), (1, 22));
    }

    #[test]
    fn blocks_take_rows_between_lines() {
        let buffer = Buffer::from_text("a\nb\nc");
        let mut map = WrapMap::default();
        let blocks = [BlockSpec { before_line: 1, rows: 2 }, BlockSpec { before_line: 3, rows: 1 }];
        map.update(&buffer, None, &blocks);
        // a, [block 0 ×2], b, c, [block 1]
        assert_eq!(map.rows(), 6);
        assert_eq!(map.first_row(1), 3);
        assert_eq!(map.text_rows(1), 3..4);
        assert_eq!(map.block_at(1), Some((0, 0)));
        assert_eq!(map.block_at(2), Some((0, 1)));
        assert_eq!(map.block_at(3), None);
        assert_eq!(map.block_at(5), Some((1, 0)));
        assert_eq!(map.row(2, &buffer).block, Some((0, 1)));
        assert_eq!(map.to_display(2, 0), (4, 0));
        assert_eq!(map.line_of_row(4), 2);
        // A block's row leads to the line below it.
        assert_eq!(map.to_offset(1, 3, &buffer), buffer.line_to_char(1));
    }
}
