use ropey::Rope;
use std::collections::VecDeque;
use std::ops::Range;

/// How many edits the buffer remembers for whatever follows it (the parser, the
/// language server). One further behind starts over from the whole text.
const EDIT_LOG: usize = 512;

/// One change, in the units the things following the text need: bytes and
/// (line, byte column) for the parser, (line, UTF-16 column) for language servers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start: (usize, usize),
    pub old_end: (usize, usize),
    pub new_end: (usize, usize),
    /// Where the replaced text was, as language servers count (line, UTF-16 column),
    /// before the change. None when they can't say it: an edit that splits or joins
    /// a "\r\n" changes lines in a way positions can't describe.
    pub lsp_range: Option<((u32, u32), (u32, u32))>,
    pub text: String,
}

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
    /// Goes up with every change, undo included (unlike `version`, which an undo
    /// takes back), so a follower knows exactly which edits it has seen.
    revision: u64,
    /// The latest edits, each with the revision it made.
    edits: VecDeque<(u64, Edit)>,
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
        Self { text: Rope::from_str(source), version: 0, saved_version: 0, revision: 0, edits: VecDeque::new() }
    }

    pub fn rope(&self) -> &Rope {
        &self.text
    }

    /// Goes up with every edit.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The edits made since `revision`, in order. None when they aren't all known
    /// (too old, or the text was replaced as a whole): start over from the text.
    pub fn edits_since(&self, revision: u64) -> Option<impl Iterator<Item = &Edit>> {
        if revision > self.revision {
            return None;
        }
        if revision < self.revision {
            let first = self.edits.front()?.0;
            if first > revision + 1 {
                return None;
            }
        }
        Some(self.edits.iter().filter(move |(r, _)| *r > revision).map(|(_, e)| e))
    }

    /// (line, byte column) of a char index.
    fn byte_point(&self, offset: usize) -> (usize, usize) {
        let line = self.text.char_to_line(offset);
        (line, self.text.char_to_byte(offset) - self.text.line_to_byte(line))
    }

    /// Whether a char index falls between the two halves of a "\r\n".
    fn splits_crlf(&self, offset: usize) -> bool {
        offset > 0 && self.char_at(offset - 1) == Some('\r') && self.char_at(offset) == Some('\n')
    }

    /// (line, UTF-16 column) of a char index.
    fn utf16_point(&self, offset: usize) -> (u32, u32) {
        let line = self.text.char_to_line(offset);
        let column = self.text.char_to_utf16_cu(offset) - self.text.char_to_utf16_cu(self.text.line_to_char(line));
        (line as u32, column as u32)
    }

    pub fn is_dirty(&self) -> bool {
        self.version != self.saved_version
    }

    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
    }

    /// Counts as having unsaved changes (a copy of a text that has them).
    pub fn mark_unsaved(&mut self) {
        self.saved_version = u64::MAX;
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
        let start_byte = self.text.char_to_byte(start);
        let (start_point, old_end) = (self.byte_point(start), self.byte_point(end));
        let lsp_range = (!self.splits_crlf(start) && !self.splits_crlf(end))
            .then(|| (self.utf16_point(start), self.utf16_point(end)));
        let old_end_byte = self.text.char_to_byte(end);
        if start < end {
            self.text.remove(start..end);
        }
        if !text.is_empty() {
            self.text.insert(start, text);
        }
        self.version += 1;
        let new_end = start + text.chars().count();
        let lsp_range = lsp_range.filter(|_| !self.splits_crlf(start) && !self.splits_crlf(new_end));
        self.revision += 1;
        let edit = Edit {
            start_byte,
            old_end_byte,
            new_end_byte: start_byte + text.len(),
            start: start_point,
            old_end,
            new_end: self.byte_point(new_end),
            lsp_range,
            text: text.to_string(),
        };
        if self.edits.len() == EDIT_LOG {
            self.edits.pop_front();
        }
        self.edits.push_back((self.revision, edit));
        new_end
    }

    /// Restores an undo snapshot along with the version it had, so undoing back to the
    /// saved text counts as saved again. It's made as the one edit between the two texts
    /// (what's the same at both ends left as it is), so what follows the text (folds,
    /// bookmarks, breakpoints, the language server) follows an undo like any other edit.
    pub fn restore_version(&mut self, text: Rope, version: u64) {
        let (old, new) = (&self.text, &text);
        let same_start = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
        let room = old.len_chars().min(new.len_chars()) - same_start;
        let same_end = old
            .chars_at(old.len_chars())
            .reversed()
            .zip(new.chars_at(new.len_chars()).reversed())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        let middle = new.slice(same_start..new.len_chars() - same_end).to_string();
        let old_end = old.len_chars() - same_end;
        self.replace(same_start..old_end, &middle);
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
    fn edits_are_logged_in_every_unit() {
        let mut buf = Buffer::from_text("é\nab");
        let start = buf.revision();
        buf.replace(3..4, "xyz\n");
        let edits: Vec<Edit> = buf.edits_since(start).unwrap().cloned().collect();
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        // "é" is two bytes: "b" starts at byte 4, column 1 of line 1.
        assert_eq!((e.start_byte, e.old_end_byte, e.new_end_byte), (4, 5, 8));
        assert_eq!((e.start, e.old_end, e.new_end), ((1, 1), (1, 2), (2, 0)));
        assert_eq!(e.lsp_range, Some(((1, 1), (1, 2))));
        assert_eq!(buf.edits_since(buf.revision()).unwrap().count(), 0);
        // An undo is the one edit back: "xyz\n" turned into "b" again, and the version it had.
        let before_undo = buf.revision();
        buf.restore_version(Rope::from_str("é\nab"), 0);
        assert_eq!(buf.to_string(), "é\nab");
        assert_eq!(buf.version(), 0);
        let undo: Vec<Edit> = buf.edits_since(before_undo).unwrap().cloned().collect();
        assert_eq!(undo.len(), 1);
        assert_eq!((undo[0].start, undo[0].old_end, undo[0].new_end), ((1, 1), (2, 0), (1, 2)));
        assert_eq!(undo[0].text, "b");
        assert_eq!(buf.edits_since(start).unwrap().count(), 2);
        // Texts that share nothing, or are the same: still right.
        buf.restore_version(Rope::from_str("other"), 7);
        assert_eq!(buf.to_string(), "other");
        buf.restore_version(Rope::from_str("other"), 8);
        assert_eq!((buf.to_string().as_str(), buf.version()), ("other", 8));
        buf.restore_version(Rope::from_str("other other"), 9);
        assert_eq!(buf.to_string(), "other other");
    }

    /// How long an undo takes in a big file, now it's found as one edit (run by hand).
    #[test]
    #[ignore]
    fn timing_undo() {
        let text = "let value = compute(index, \"text\");\n".repeat(150_000);
        let mut buf = Buffer::from_text(&text);
        let middle = buf.len_chars() / 2;
        let before = buf.rope().clone();
        buf.replace(middle..middle, "x");
        let started = std::time::Instant::now();
        buf.restore_version(before, 0);
        println!("undo in {} chars: {:?}", buf.len_chars(), started.elapsed());
    }

    #[test]
    fn a_server_replaying_the_edits_gets_the_same_text() {
        let start = "fn é() {\n    let 😀 = 1;\n}\n";
        let mut buf = Buffer::from_text(start);
        let mut seed = 7u64;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n.max(1)
        };
        let pieces = ["", "x", "😀", "é\n", "\n\n", "ab\r\ncd", "\r", "\n"];
        for _ in 0..300 {
            let a = next(buf.len_chars() + 1);
            let b = (a + next(4)).min(buf.len_chars());
            buf.replace(a..b, pieces[next(pieces.len())]);
        }
        // As a server does: each change in turn, at a line and a UTF-16 column, or the
        // whole text when an edit can't be placed that way (as the editor sends it then).
        let mut server = Buffer::from_text(start);
        let mut whole = 0;
        for (i, e) in buf.edits_since(0).unwrap().enumerate() {
            let Some((from, to)) = e.lsp_range else {
                whole += 1;
                let mut replay = Buffer::from_text(start);
                for e in buf.edits_since(0).unwrap().take(i + 1) {
                    let (a, b) = (replay.rope().byte_to_char(e.start_byte), replay.rope().byte_to_char(e.old_end_byte));
                    replay.replace(a..b, &e.text);
                }
                server = replay;
                continue;
            };
            let at = |(line, col): (u32, u32)| {
                server.offset(line as usize, server.utf16_to_column(line as usize, col as usize))
            };
            let range = at(from)..at(to);
            server.replace(range, &e.text);
        }
        assert_eq!(server.to_string(), buf.to_string());
        assert!(whole > 0 && whole < 100, "{whole} edits needed the whole text");
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
