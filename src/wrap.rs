//! Word wrap: splits long lines into screen rows that fit the editor's width.
//! Code fonts are monospaced, so widths are counted in characters.
//!
//! With wrapping off every line is one row, and all of this is a no-op.

use crate::buffer::Buffer;
use std::ops::Range;

/// Continuation rows keep the line's indentation, unless that would leave less room than this.
const MIN_ROOM: usize = 20;

/// Which rows each line takes on screen.
pub struct WrapMap {
    /// Characters per row, or None when wrapping is off.
    width: Option<usize>,
    version: u64,
    lines: usize,
    /// For each line, the columns where its rows start; the first is always 0.
    starts: Vec<Vec<usize>>,
    /// For each line, how far its continuation rows are indented, in columns.
    indents: Vec<usize>,
    /// The first row of each line, then the total number of rows.
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
}

impl Default for WrapMap {
    fn default() -> Self {
        Self {
            width: None,
            version: u64::MAX,
            lines: 1,
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

    /// Brings the map up to date with the text and the row width (None to stop wrapping).
    pub fn update(&mut self, buffer: &Buffer, width: Option<usize>) {
        let width = width.map(|w| w.max(MIN_ROOM));
        if width == self.width && buffer.version() == self.version && buffer.len_lines() == self.lines {
            return;
        }
        self.width = width;
        self.version = buffer.version();
        self.lines = buffer.len_lines();
        self.starts.clear();
        self.indents.clear();
        self.first_rows.clear();
        let Some(width) = width else { return };
        let mut row = 0;
        for line in buffer.rope().lines() {
            let text: String = line.chars().filter(|c| *c != '\n' && *c != '\r').collect();
            let (starts, indent) = wrap_line(&text, width);
            self.first_rows.push(row);
            row += starts.len();
            self.starts.push(starts);
            self.indents.push(indent);
        }
        // Ropey doesn't yield the empty line after a final line break; keep the counts in step.
        while self.first_rows.len() < self.lines {
            self.first_rows.push(row);
            row += 1;
            self.starts.push(vec![0]);
            self.indents.push(0);
        }
        self.first_rows.truncate(self.lines);
        self.starts.truncate(self.lines);
        self.indents.truncate(self.lines);
        self.first_rows.push(row);
    }

    pub fn rows(&self) -> usize {
        if self.is_on() { *self.first_rows.last().unwrap_or(&1) } else { self.lines }
    }

    /// The first row of `line`. Past the last line, the number of rows.
    pub fn first_row(&self, line: usize) -> usize {
        if self.is_on() { self.first_rows[line.min(self.lines)] } else { line.min(self.lines) }
    }

    pub fn line_of_row(&self, row: usize) -> usize {
        if !self.is_on() {
            return row.min(self.lines.saturating_sub(1));
        }
        self.first_rows.partition_point(|&r| r <= row).saturating_sub(1).min(self.lines.saturating_sub(1))
    }

    pub fn row(&self, row: usize, buffer: &Buffer) -> Row {
        let line = self.line_of_row(row);
        if !self.is_on() {
            return Row { line, cols: 0..buffer.line_len(line), indent: 0, last: true };
        }
        let starts = &self.starts[line];
        let i = (row - self.first_rows[line]).min(starts.len() - 1);
        let last = i + 1 == starts.len();
        let end = if last { buffer.line_len(line) } else { starts[i + 1] };
        Row { line, cols: starts[i]..end, indent: if i > 0 { self.indents[line] } else { 0 }, last }
    }

    /// Where a (line, column) shows: its row, and its column on that row counting the indent.
    pub fn to_display(&self, line: usize, col: usize) -> (usize, usize) {
        if !self.is_on() {
            return (line, col);
        }
        let line = line.min(self.lines - 1);
        let starts = &self.starts[line];
        let i = starts.partition_point(|&s| s <= col).saturating_sub(1);
        let indent = if i > 0 { self.indents[line] } else { 0 };
        (self.first_rows[line] + i, col - starts[i] + indent)
    }

    /// The char offset shown at `col` on `row`. Past the end of a wrapped row, the caret
    /// stays on that row rather than jumping to the start of the next one.
    pub fn to_offset(&self, row: usize, col: usize, buffer: &Buffer) -> usize {
        let row = self.row(row.min(self.rows().saturating_sub(1)), buffer);
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
        map.update(&buffer, Some(20));
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

        map.update(&buffer, None);
        assert_eq!(map.rows(), 4);
        assert_eq!(map.to_display(1, 22), (1, 22));
    }
}
