use ropey::Rope;
use std::ops::Range;

/// Text storage for one open file. Backed by a rope, so edits stay fast
/// even on very large files.
///
/// All positions are char indices. Out-of-range input is clamped instead of
/// panicking, because edits can come from places the editor doesn't control
/// (IME, language servers, stale undo steps).
#[derive(Clone)]
pub struct Buffer {
    text: Rope,
    version: u64,
    saved_version: u64,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    pub fn new() -> Self {
        Self::from_text("")
    }

    pub fn from_text(source: &str) -> Self {
        Self { text: Rope::from_str(source), version: 0, saved_version: 0 }
    }

    pub fn rope(&self) -> &Rope {
        &self.text
    }

    /// Goes up with every edit.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn is_dirty(&self) -> bool {
        self.version != self.saved_version
    }

    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
    }

    pub fn len_chars(&self) -> usize {
        self.text.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.text.len_lines()
    }

    /// Replaces `range` with `text` and returns the char index just after
    /// the inserted text.
    pub fn replace(&mut self, range: Range<usize>, text: &str) -> usize {
        let end = range.end.min(self.len_chars());
        let start = range.start.min(end);
        if start < end {
            self.text.remove(start..end);
        }
        if !text.is_empty() {
            self.text.insert(start, text);
        }
        self.version += 1;
        start + text.chars().count()
    }

    /// Restores an undo snapshot along with the version it had, so undoing back to the
    /// saved text counts as saved again.
    pub fn restore_version(&mut self, text: Rope, version: u64) {
        self.text = text;
        self.version = version;
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.len_chars());
        let start = range.start.min(end);
        self.text.slice(start..end).to_string()
    }

    pub fn char_at(&self, idx: usize) -> Option<char> {
        (idx < self.len_chars()).then(|| self.text.char(idx))
    }

    /// The text of line `idx` without its line break.
    pub fn line_text(&self, idx: usize) -> String {
        if idx >= self.len_lines() {
            return String::new();
        }
        let mut line = self.text.line(idx).to_string();
        while line.ends_with(['\n', '\r']) {
            line.pop();
        }
        line
    }

    /// Length of line `idx` in chars, not counting its line break.
    pub fn line_len(&self, idx: usize) -> usize {
        if idx >= self.len_lines() {
            return 0;
        }
        let line = self.text.line(idx);
        let mut len = line.len_chars();
        while len > 0 && matches!(line.char(len - 1), '\n' | '\r') {
            len -= 1;
        }
        len
    }

    pub fn line_to_char(&self, line: usize) -> usize {
        self.text.line_to_char(line.min(self.len_lines()))
    }

    pub fn line_to_byte(&self, line: usize) -> usize {
        self.text.line_to_byte(line.min(self.len_lines()))
    }

    /// (line, column) for a char index, both zero-based.
    pub fn point(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.len_chars());
        let line = self.text.char_to_line(offset);
        (line, offset - self.text.line_to_char(line))
    }

    /// Char index for a (line, column), clamping both to the text.
    pub fn offset(&self, line: usize, column: usize) -> usize {
        let line = line.min(self.len_lines().saturating_sub(1));
        self.line_to_char(line) + column.min(self.line_len(line))
    }

    /// Column (in chars) of a UTF-16 position on `line`, as language servers count.
    pub fn utf16_to_column(&self, line: usize, utf16: usize) -> usize {
        let mut units = 0;
        for (column, c) in self.line_text(line).chars().enumerate() {
            if units >= utf16 {
                return column;
            }
            units += c.len_utf16();
        }
        self.line_len(line)
    }

    pub fn column_to_utf16(&self, line: usize, column: usize) -> usize {
        self.line_text(line).chars().take(column).map(char::len_utf16).sum()
    }

    pub fn char_to_utf16(&self, offset: usize) -> usize {
        self.text.char_to_utf16_cu(offset.min(self.len_chars()))
    }

    pub fn utf16_to_char(&self, offset: usize) -> usize {
        self.text.utf16_cu_to_char(offset.min(self.text.len_utf16_cu()))
    }
}

impl std::fmt::Display for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for chunk in self.text.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_bump_version_and_dirty() {
        let mut buf = Buffer::from_text("hello");
        assert!(!buf.is_dirty());
        buf.replace(5..5, " world");
        buf.replace(0..6, "");
        assert_eq!(buf.to_string(), "world");
        assert_eq!(buf.version(), 2);
        assert!(buf.is_dirty());
        buf.mark_saved();
        assert!(!buf.is_dirty());
    }

    #[test]
    #[allow(clippy::reversed_empty_ranges)] // the reversed range is what's being tested
    fn out_of_range_edits_are_clamped() {
        let mut buf = Buffer::from_text("abc");
        let end = buf.replace(10..20, "!");
        assert_eq!(buf.to_string(), "abc!");
        assert_eq!(end, 4);
        buf.replace(2..1, "");
        assert_eq!(buf.to_string(), "abc!");
    }

    #[test]
    fn lines_exclude_line_breaks() {
        let buf = Buffer::from_text("one\r\ntwo\nthree");
        assert_eq!(buf.len_lines(), 3);
        assert_eq!(buf.line_text(0), "one");
        assert_eq!(buf.line_len(0), 3);
        assert_eq!(buf.line_text(2), "three");
        assert_eq!(buf.line_text(9), "");
    }

    #[test]
    fn points_and_offsets_round_trip() {
        let buf = Buffer::from_text("fn main() {\n    println!();\n}");
        let offset = buf.offset(1, 4);
        assert_eq!(buf.point(offset), (1, 4));
        assert_eq!(buf.offset(1, 99), buf.offset(1, 15));
        assert_eq!(buf.offset(99, 0), buf.line_to_char(2));
    }

    #[test]
    fn utf16_columns_round_trip() {
        let buf = Buffer::from_text("x\n😀ab");
        assert_eq!(buf.utf16_to_column(1, 3), 2);
        assert_eq!(buf.column_to_utf16(1, 2), 3);
        assert_eq!(buf.utf16_to_column(1, 99), 3);
    }

    #[test]
    fn utf16_conversion_handles_wide_chars() {
        let buf = Buffer::from_text("a😀b");
        assert_eq!(buf.char_to_utf16(2), 3);
        assert_eq!(buf.utf16_to_char(3), 2);
    }
}
