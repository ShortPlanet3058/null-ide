//! Which rows things take on screen: lines split by word wrap, and blocks — rows
//! between lines that aren't text, like a line being typed to the AI, lines it
//! removed, or a note. Code fonts are monospaced, so widths are counted in characters.
//!
//! With wrapping off and no blocks every line is one row, and all of this is a no-op.

use crate::buffer::Buffer;
use crate::editor::TAB_SIZE;
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
    /// The buffer revision it matches.
    revision: u64,
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
    /// Lines folded away: they take no rows. Sorted, not overlapping.
    hidden: Vec<Range<usize>>,
    /// The hidden lines changed since the rows were last placed.
    hidden_changed: bool,
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
            revision: u64::MAX,
            lines: 1,
            blocks: Vec::new(),
            general: false,
            block_rows: Vec::new(),
            starts: Vec::new(),
            indents: Vec::new(),
            first_rows: Vec::new(),
            hidden: Vec::new(),
            hidden_changed: false,
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

    /// Hides these lines (folded code), from the next update.
    pub fn set_hidden(&mut self, hidden: Vec<Range<usize>>) {
        if hidden != self.hidden {
            self.hidden = hidden;
            self.hidden_changed = true;
        }
    }

    pub fn is_hidden(&self, line: usize) -> bool {
        let i = self.hidden.partition_point(|r| r.end <= line);
        self.hidden.get(i).is_some_and(|r| r.contains(&line))
    }

    /// Brings the map up to date with the text, the row width (None to stop wrapping)
    /// and the blocks between lines. After typing, only the edited lines are wrapped again.
    pub fn update(&mut self, buffer: &Buffer, width: Option<usize>, blocks: &[BlockSpec]) {
        let width = width.map(|w| w.max(MIN_ROOM));
        let same_layout = width == self.width && blocks == self.blocks.as_slice();
        if same_layout && !self.hidden_changed && buffer.revision() == self.revision && buffer.len_lines() == self.lines
        {
            return;
        }
        let general = width.is_some() || !blocks.is_empty() || !self.hidden.is_empty();
        if same_layout && self.general && general && self.rewrap_edits(buffer) {
            self.revision = buffer.revision();
            self.hidden_changed = false;
            self.place_rows();
            return;
        }
        self.hidden_changed = false;
        self.width = width;
        self.revision = buffer.revision();
        self.lines = buffer.len_lines();
        self.blocks = blocks.to_vec();
        self.starts.clear();
        self.indents.clear();
        self.first_rows.clear();
        self.block_rows.clear();
        self.general = general;
        if !self.general {
            return;
        }
        match width {
            Some(width) => {
                let mut texts = buffer.rope().lines();
                for _ in 0..self.lines {
                    // Ropey may not yield the empty line after a final line break.
                    let text: String = texts.next().map(line_chars).unwrap_or_default();
                    let (starts, indent) = wrap_line(&text, width);
                    self.starts.push(starts);
                    self.indents.push(indent);
                }
            }
            None => {
                self.starts = vec![vec![0]; self.lines];
                self.indents = vec![0; self.lines];
            }
        }
        self.place_rows();
    }

    /// Follows the buffer's edits since the last update, wrapping again only the lines
    /// they touched. False when it can't (the edits aren't known): wrap everything.
    fn rewrap_edits(&mut self, buffer: &Buffer) -> bool {
        let Some(edits) = buffer.edits_since(self.revision) else { return false };
        // Lines to wrap again, in the text as it is now.
        let mut dirty: Vec<Range<usize>> = Vec::new();
        for edit in edits {
            let (start, old_end, new_end) = (edit.start.0, edit.old_end.0, edit.new_end.0);
            if old_end >= self.starts.len() {
                return false;
            }
            let added = new_end as isize - old_end as isize;
            self.starts.splice(start..=old_end, (start..=new_end).map(|_| vec![0]));
            self.indents.splice(start..=old_end, (start..=new_end).map(|_| 0));
            for range in &mut dirty {
                if range.start > old_end {
                    *range = (range.start as isize + added) as usize..(range.end as isize + added) as usize;
                } else if range.end > start {
                    // Overlapping the edit: from the edit's start (or before) to past its new lines.
                    range.start = range.start.min(start);
                    range.end = (range.end as isize + added).max(new_end as isize + 1) as usize;
                }
            }
            dirty.push(start..new_end + 1);
        }
        self.lines = buffer.len_lines();
        if self.starts.len() != self.lines {
            return false;
        }
        if let Some(width) = self.width {
            for range in dirty {
                for line in range.start.min(self.lines)..range.end.min(self.lines) {
                    let text = line_chars(buffer.rope().line(line));
                    (self.starts[line], self.indents[line]) = wrap_line(&text, width);
                }
            }
        }
        true
    }

    /// Where each line's rows and each block's rows go, from the rows each line takes.
    fn place_rows(&mut self) {
        self.first_rows.clear();
        self.block_rows.clear();
        let blocks = &self.blocks;
        let mut order: Vec<usize> = (0..blocks.len()).collect();
        order.sort_by_key(|&i| blocks[i].before_line);
        let mut pending = order.into_iter().peekable();
        let mut hidden = self.hidden.iter().peekable();
        let mut row = 0;
        for line in 0..self.lines {
            while let Some(&b) = pending.peek().filter(|&&b| blocks[b].before_line <= line) {
                self.block_rows.push((row, b));
                row += blocks[b].rows;
                pending.next();
            }
            self.first_rows.push(row);
            while hidden.peek().is_some_and(|r| r.end <= line) {
                hidden.next();
            }
            if !hidden.peek().is_some_and(|r| r.contains(&line)) {
                row += self.starts[line].len();
            }
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
        if self.is_hidden(line) {
            return start..start;
        }
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

    /// Where a (line, column) shows: its row, and its screen column on that row counting
    /// the indent (and tabs at their full width).
    pub fn to_display(&self, line: usize, col: usize, buffer: &Buffer) -> (usize, usize) {
        let chars = |from: usize, to: usize| {
            buffer.rope().line(line.min(buffer.len_lines() - 1)).chars_at(from).take(to.saturating_sub(from))
        };
        if !self.general {
            // Far along a very long line, a column a character: counting them all, several
            // times a frame, was most of the frame.
            if col > LONG_LINE {
                return (line, col);
            }
            return (line, columns(chars(0, col)));
        }
        let line = line.min(self.lines - 1);
        let starts = &self.starts[line];
        let i = starts.partition_point(|&s| s <= col).saturating_sub(1);
        let indent = if i > 0 { self.indents[line] } else { 0 };
        (self.first_rows[line] + i, columns(chars(starts[i], col)) + indent)
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
        let shown = buffer.rope().line(row.line).chars_at(row.cols.start).take(row.cols.len());
        let col = row.cols.start.saturating_add(char_at_column(shown, col.saturating_sub(row.indent)));
        let max = if row.last { row.cols.end } else { row.cols.end.saturating_sub(1).max(row.cols.start) };
        buffer.offset(row.line, col.min(max))
    }
}

/// A line's text without its line break.
fn line_chars(line: ropey::RopeSlice) -> String {
    line.chars().filter(|c| *c != '\n' && *c != '\r').collect()
}

/// How many columns a char takes at column `col` of a row: a tab reaches the next tab
/// stop (stops count from the row's start), anything else takes one.
pub fn char_columns(c: char, col: usize) -> usize {
    if c == '\t' { TAB_SIZE - col % TAB_SIZE } else { 1 }
}

/// The columns `chars` take on screen.
/// Lines longer than this (in columns) are drawn only around the view, a column a
/// character past it, so a minified file's one long line stays quick to show and edit.
pub const LONG_LINE: usize = 2_000;

fn columns(chars: impl Iterator<Item = char>) -> usize {
    chars.fold(0, |col, c| col + char_columns(c, col))
}

/// Which of `chars` shows at screen column `target`: past the end, their count; on a
/// tab, whichever edge of it is nearer.
fn char_at_column(chars: impl Iterator<Item = char>, target: usize) -> usize {
    let mut col = 0;
    for (i, c) in chars.enumerate() {
        let w = char_columns(c, col);
        if target < col + w {
            return if target - col > (w - 1) / 2 && w > 1 { i + 1 } else { i };
        }
        col += w;
    }
    usize::MAX
}

/// Where a line's rows start, and how far its continuation rows are indented.
/// Breaks go after a space when one fits, so words stay whole; a word longer than
/// the row is cut where the row ends. Widths are screen columns: a tab counts to its stop.
fn wrap_line(text: &str, width: usize) -> (Vec<usize>, usize) {
    let chars: Vec<char> = text.chars().collect();
    let mut starts = vec![0];
    if chars.len() <= width && !chars.contains(&'\t') {
        return (starts, 0);
    }
    let lead = chars.iter().take_while(|c| **c == ' ' || **c == '\t').count();
    let lead_columns = columns(chars[..lead].iter().copied());
    let indent = if lead_columns + MIN_ROOM <= width { lead_columns } else { 0 };
    let mut start = 0;
    loop {
        let room = if starts.len() == 1 { width } else { width - indent };
        // How many chars from `start` fit in the room.
        let (mut col, mut fit) = (0, start);
        while fit < chars.len() {
            let w = char_columns(chars[fit], col);
            if col + w > room {
                break;
            }
            col += w;
            fit += 1;
        }
        if fit >= chars.len() {
            break;
        }
        let limit = fit.max(start + 1);
        let after_space = |i: &usize| chars[*i - 1] == ' ' && chars[*i] != ' ';
        let after_punct = |i: &usize| matches!(chars[*i - 1], ',' | ';' | '(' | '[' | '{' | '.' | '/' | '-');
        let earliest = start + (limit - start) / 3;
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
    fn edits_rewrap_only_their_lines_and_match_a_fresh_map() {
        let long = "word ".repeat(12);
        let mut buffer = Buffer::from_text(&format!("a\n{long}\nb\n{long}\nc"));
        let blocks = [BlockSpec { before_line: 3, rows: 2 }];
        let mut map = WrapMap::default();
        map.update(&buffer, Some(20), &blocks);
        // Join two lines, split one, type on a wrapped one: three edits before the next update.
        let b = buffer.line_to_char(2);
        buffer.replace(b - 1..b, "");
        let at = buffer.offset(0, 1);
        buffer.replace(at..at, "\nnew line\n");
        let at = buffer.offset(3, 3);
        buffer.replace(at..at, &"more words ".repeat(4));
        // Then lines removed across the edited ones.
        let (from, to) = (buffer.offset(2, 2), buffer.offset(4, 1));
        buffer.replace(from..to, "");
        map.update(&buffer, Some(20), &blocks);
        let mut fresh = WrapMap::default();
        fresh.update(&buffer, Some(20), &blocks);
        assert_eq!(map.lines, fresh.lines);
        assert_eq!(map.starts, fresh.starts);
        assert_eq!(map.indents, fresh.indents);
        assert_eq!(map.first_rows, fresh.first_rows);
        assert_eq!(map.block_rows, fresh.block_rows);
    }

    #[test]
    fn folded_lines_take_no_rows() {
        let buffer = Buffer::from_text("fn a() {\n    one\n    two\n}\nend");
        let mut map = WrapMap::default();
        map.set_hidden(vec![1..3]);
        map.update(&buffer, None, &[]);
        assert_eq!(map.rows(), 3);
        // Row 1 is the closing line, row 2 the last.
        assert_eq!(map.line_of_row(1), 3);
        assert_eq!(map.to_display(3, 0, &buffer), (1, 0));
        assert_eq!(map.to_offset(2, 0, &buffer), buffer.line_to_char(4));
        assert!(map.text_rows(2).is_empty());
        // Unfolding brings them back.
        map.set_hidden(Vec::new());
        map.update(&buffer, None, &[]);
        assert_eq!(map.rows(), 5);
    }

    #[test]
    fn tabs_take_their_full_width() {
        let buffer = Buffer::from_text("\tx\n    y\na\tb");
        let map = WrapMap::default();
        // Under the tab's x, the spaces' y: both at column 4.
        assert_eq!(map.to_display(0, 1, &buffer), (0, 4));
        assert_eq!(map.to_display(1, 4, &buffer), (1, 4));
        assert_eq!(map.to_offset(0, 4, &buffer), buffer.offset(0, 1));
        // Inside a tab, the nearer edge.
        assert_eq!(map.to_offset(0, 1, &buffer), buffer.offset(0, 0));
        assert_eq!(map.to_offset(0, 3, &buffer), buffer.offset(0, 1));
        // A tab after a char reaches the same stop.
        assert_eq!(map.to_display(2, 2, &buffer), (2, 4));
        // Wrapping counts a tab's columns.
        let (starts, indent) = wrap_line(&format!("\t\t{}", "word ".repeat(6)), 24);
        assert_eq!(indent, 0);
        assert!(starts[1] <= 2 + 16, "{starts:?}");
    }

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
        assert_eq!(map.to_display(1, 22, &buffer), (2, 2));
        assert_eq!(map.to_offset(2, 2, &buffer), buffer.offset(1, 22));
        // Past the end of a wrapped row: stays on that row.
        assert_eq!(map.to_offset(1, 50, &buffer), buffer.offset(1, 19));
        // Past the end of a line's last row: the end of the line.
        assert_eq!(map.to_offset(3, 50, &buffer), buffer.offset(1, 50));

        map.update(&buffer, None, &[]);
        assert_eq!(map.rows(), 4);
        assert_eq!(map.to_display(1, 22, &buffer), (1, 22));
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
        assert_eq!(map.to_display(2, 0, &buffer), (4, 0));
        assert_eq!(map.line_of_row(4), 2);
        // A block's row leads to the line below it.
        assert_eq!(map.to_offset(1, 3, &buffer), buffer.line_to_char(1));
    }
}
