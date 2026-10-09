//! Vim's keys, for fingers that know them (the Vim keymap in Settings). Normal mode moves
//! and edits, Insert types, Visual selects; ⌘ shortcuts stay Null's own. Keys are taken
//! before any binding sees them, and only by the editor that has the keyboard.

use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "Normal",
            Mode::Insert => "Insert",
            Mode::Visual => "Visual",
            Mode::VisualLine => "Visual Line",
        }
    }

    fn visual(self) -> bool {
        matches!(self, Mode::Visual | Mode::VisualLine)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
}

impl Operator {
    fn of(c: char) -> Option<Self> {
        Some(match c {
            'd' => Operator::Delete,
            'c' => Operator::Change,
            'y' => Operator::Yank,
            '>' => Operator::Indent,
            '<' => Operator::Outdent,
            _ => return None,
        })
    }
}

/// Text yanked or deleted, and whether it's whole lines.
#[derive(Clone, Debug)]
struct Register {
    text: String,
    linewise: bool,
}

/// Where a motion goes, and how an operator takes it: whole lines, or up to it (and the
/// character at it too, `inclusive`).
#[derive(Clone, Copy, Debug)]
struct Motion {
    to: usize,
    linewise: bool,
    inclusive: bool,
}

/// One editor's Vim state.
#[derive(Default)]
pub struct Vim {
    pub mode: Mode,
    count: Option<usize>,
    /// An operator waiting for its motion, with the count typed before it.
    operator: Option<(Operator, Option<usize>)>,
    /// A key waiting for the next: `g` (gg), `r` (the character to put).
    pending: Option<char>,
    register: Option<Register>,
    /// Visual mode: where it started, and the end that moves.
    anchor: usize,
    head: usize,
    /// The column `j` and `k` keep to (`usize::MAX` after `$`: the line's end).
    goal: Option<usize>,
}

/// A key as Vim reads it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Key {
    Char(char),
    Ctrl(char),
    Named(String),
}

/// Whether the Vim keymap is in use.
pub fn on(cx: &App) -> bool {
    cx.try_global::<Settings>().is_some_and(|s| s.keymap == crate::keymap::Keymap::Vim)
}

/// Takes the keys of `editor` (while it has the keyboard) before bindings do.
pub(super) fn listen(cx: &mut Context<Editor>) -> Subscription {
    let editor = cx.entity().downgrade();
    cx.intercept_keystrokes(move |event, window, cx| {
        if !on(cx) {
            return;
        }
        let keystroke = event.keystroke.clone();
        let taken = editor
            .update(cx, |editor, cx| editor.focus_handle.is_focused(window) && editor.vim_key(&keystroke, window, cx))
            .unwrap_or(false);
        if taken {
            cx.stop_propagation();
        }
    })
}

fn class(c: char, big: bool) -> u8 {
    if c.is_whitespace() {
        0
    } else if big || c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

impl Editor {
    /// Whether the caret is a block (Vim, out of Insert mode).
    pub fn block_caret(&self, cx: &App) -> bool {
        on(cx) && self.vim.mode != Mode::Insert && self.preview.is_none()
    }

    /// The mode to show, when Vim's keys are in use.
    pub fn vim_mode(&self, cx: &App) -> Option<Mode> {
        on(cx).then_some(self.vim.mode)
    }

    /// Where the caret shows: in Visual mode, on the character at the moving end (the
    /// selection itself ends after it).
    pub fn shown_caret(&self) -> usize {
        if self.vim.mode.visual() && !self.selection.is_empty() {
            self.vim.head.min(self.buffer.len_chars())
        } else {
            self.selection.head
        }
    }

    /// A keystroke, for Vim: true when it was taken.
    pub(super) fn vim_key(&mut self, keystroke: &gpui::Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let m = keystroke.modifiers;
        if m.platform || m.function || self.preview.is_some() || self.reading {
            return false;
        }
        if self.vim.mode == Mode::Insert {
            let leaves = (keystroke.key == "escape" && !m.control && !m.alt)
                || (m.control && matches!(keystroke.key.as_str(), "[" | "c"));
            if leaves {
                self.vim_leave_insert(cx);
            }
            return leaves;
        }
        if m.alt {
            return false;
        }
        let key = if m.control {
            match keystroke.key.chars().next() {
                Some(c) if keystroke.key.chars().count() == 1 => Key::Ctrl(c),
                _ => return false,
            }
        } else if let Some(c) = keystroke.key_char.as_deref().filter(|k| k.chars().count() == 1).and_then(|k| k.chars().next())
        {
            Key::Char(c)
        } else {
            Key::Named(keystroke.key.clone())
        };
        self.vim_input(key, window, cx)
    }

    /// Text that came by the input method (an accent key, `^` on a French keyboard) while
    /// not typing: read as keys.
    pub(super) fn vim_takes_text(&self, cx: &App) -> bool {
        on(cx) && self.vim.mode != Mode::Insert && self.preview.is_none() && !self.reading
    }

    pub(super) fn vim_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        for c in text.chars() {
            self.vim_input(Key::Char(c), window, cx);
        }
    }

    fn vim_leave_insert(&mut self, cx: &mut Context<Self>) {
        self.close_completion(cx);
        self.single_cursor();
        let head = self.selection.head;
        let (_, column) = self.buffer.point(head);
        // Back onto the last character typed, as Vim does.
        let at = if column > 0 { head - 1 } else { head };
        self.vim.mode = Mode::Normal;
        self.vim_place(at, cx);
    }

    /// Where the caret may be in Normal mode: on a character, not after a line's last.
    fn vim_clamp(&self, offset: usize) -> usize {
        let (line, _) = self.buffer.point(offset);
        let start = self.buffer.line_to_char(line);
        let len = self.buffer.line_len(line);
        if len == 0 { start } else { offset.clamp(start, start + len - 1) }
    }

    /// The caret at `offset` (in Normal mode, on a character).
    fn vim_place(&mut self, offset: usize, cx: &mut Context<Self>) {
        let at = self.vim_clamp(offset);
        self.selection = Selection::caret(at);
        self.goal_column = None;
        self.touch(cx);
    }

    /// The last line with text (ropey counts an empty one after a final line break).
    fn vim_last_line(&self) -> usize {
        let lines = self.buffer.len_lines();
        if lines > 1 && self.buffer.line_to_char(lines - 1) == self.buffer.len_chars() { lines - 2 } else { lines - 1 }
    }

    fn line_start(&self, line: usize) -> usize {
        self.buffer.line_to_char(line)
    }

    fn line_end(&self, line: usize) -> usize {
        self.buffer.line_to_char(line) + self.buffer.line_len(line)
    }

    fn first_non_blank(&self, line: usize) -> usize {
        let start = self.line_start(line);
        let blank = self.buffer.line_text(line).chars().take_while(|c| *c == ' ' || *c == '\t').count();
        start + blank
    }

    fn empty_line_at(&self, offset: usize) -> bool {
        let (line, column) = self.buffer.point(offset);
        column == 0 && self.buffer.line_len(line) == 0
    }

    fn next_word_start(&self, from: usize, big: bool) -> usize {
        let rope = self.buffer.rope();
        let len = rope.len_chars();
        let mut i = from;
        if i >= len {
            return len;
        }
        let first = class(rope.char(i), big);
        if first != 0 {
            while i < len && class(rope.char(i), big) == first {
                i += 1;
            }
        }
        while i < len && rope.char(i).is_whitespace() {
            // An empty line is a word of its own.
            if i > from && self.empty_line_at(i) {
                return i;
            }
            i += 1;
        }
        i
    }

    fn word_end(&self, from: usize, big: bool) -> usize {
        let rope = self.buffer.rope();
        let len = rope.len_chars();
        let mut i = from + 1;
        while i < len && rope.char(i).is_whitespace() {
            i += 1;
        }
        if i >= len {
            return len.saturating_sub(1);
        }
        let k = class(rope.char(i), big);
        while i + 1 < len && class(rope.char(i + 1), big) == k {
            i += 1;
        }
        i
    }

    fn word_start_back(&self, from: usize, big: bool) -> usize {
        let rope = self.buffer.rope();
        if from == 0 {
            return 0;
        }
        let mut i = from - 1;
        while i > 0 && rope.char(i).is_whitespace() {
            if self.empty_line_at(i) {
                return i;
            }
            i -= 1;
        }
        let k = class(rope.char(i), big);
        while i > 0 && k != 0 && class(rope.char(i - 1), big) == k {
            i -= 1;
        }
        i
    }

    /// Where `key` moves the caret, `count` times; None when it's no motion.
    fn vim_motion(&mut self, key: &Key, count: Option<usize>, operator: bool) -> Option<Motion> {
        let n = count.unwrap_or(1).max(1);
        let at = if self.vim.mode.visual() { self.vim.head } else { self.selection.head };
        let (line, column) = self.buffer.point(at);
        let last = self.vim_last_line();
        let exclusive = |to| Motion { to, linewise: false, inclusive: false };
        let inclusive = |to| Motion { to, linewise: false, inclusive: true };
        let lines = |to_line: usize, this: &mut Self| {
            let to_line = to_line.min(last);
            // The column kept by j and k (to the line's end after $).
            let goal = *this.vim.goal.get_or_insert(column);
            let len = this.buffer.line_len(to_line);
            let col = if goal == usize::MAX { len.saturating_sub(1) } else { goal.min(len.saturating_sub(1)) };
            Motion { to: this.line_start(to_line) + col, linewise: true, inclusive: false }
        };
        let keeps_goal = matches!(key, Key::Char('j' | 'k') | Key::Ctrl('d' | 'u' | 'f' | 'b'));
        if !keeps_goal {
            self.vim.goal = None;
        }
        let page = self.page_lines().max(1) as usize;
        Some(match key {
            Key::Char('h') => exclusive(at.saturating_sub(n).max(self.line_start(line))),
            Key::Char('l' | ' ') => {
                // With an operator, up to the line's end (dl at the last character takes it).
                let end = if operator {
                    self.line_end(line)
                } else {
                    self.line_end(line).saturating_sub(1).max(self.line_start(line))
                };
                exclusive((at + n).min(end))
            }
            Key::Char('j') => lines(line + n, self),
            Key::Char('k') => lines(line.saturating_sub(n), self),
            Key::Char('+') => {
                let to = (line + n).min(last);
                Motion { to: self.first_non_blank(to), linewise: true, inclusive: false }
            }
            Key::Char('-') => {
                let to = line.saturating_sub(n);
                Motion { to: self.first_non_blank(to), linewise: true, inclusive: false }
            }
            Key::Ctrl('d') => lines(line + page / 2, self),
            Key::Ctrl('u') => lines(line.saturating_sub(page / 2), self),
            Key::Ctrl('f') => lines(line + page, self),
            Key::Ctrl('b') => lines(line.saturating_sub(page), self),
            Key::Char(c @ ('w' | 'W')) => {
                let big = *c == 'W';
                let (mut from, mut to) = (at, at);
                for _ in 0..n {
                    from = to;
                    to = self.next_word_start(to, big);
                }
                // dw on a line's last word stops at its end, as Vim does: the break stays.
                let from_line = self.buffer.point(from).0;
                if operator && self.buffer.point(to).0 > from_line {
                    return Some(exclusive(self.line_end(from_line).max(at)));
                }
                exclusive(to)
            }
            Key::Char(c @ ('e' | 'E')) => {
                let big = *c == 'E';
                let mut to = at;
                for _ in 0..n {
                    to = self.word_end(to, big);
                }
                inclusive(to)
            }
            Key::Char(c @ ('b' | 'B')) => {
                let big = *c == 'B';
                let mut to = at;
                for _ in 0..n {
                    to = self.word_start_back(to, big);
                }
                exclusive(to)
            }
            Key::Char('0') => exclusive(self.line_start(line)),
            Key::Char('^') => exclusive(self.first_non_blank(line)),
            Key::Char('$') => {
                self.vim.goal = Some(usize::MAX);
                let to_line = (line + n - 1).min(last);
                inclusive(self.line_end(to_line).saturating_sub(1).max(self.line_start(to_line)))
            }
            Key::Char('G') => {
                let to = count.map_or(last, |c| c.saturating_sub(1).min(last));
                Motion { to: self.first_non_blank(to), linewise: true, inclusive: false }
            }
            _ => return None,
        })
    }

    /// One key in Normal or Visual mode. True when Vim took it.
    pub(super) fn vim_input(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // The arrows and such, as the letters Vim has for them.
        let key = match key {
            Key::Named(k) => match k.as_str() {
                "left" | "backspace" => Key::Char('h'),
                "right" => Key::Char('l'),
                "down" => Key::Char('j'),
                "up" => Key::Char('k'),
                "enter" => Key::Char('+'),
                "home" => Key::Char('0'),
                "end" => Key::Char('$'),
                "delete" => Key::Char('x'),
                _ => Key::Named(k),
            },
            other => other,
        };
        if !self.extra.is_empty() {
            self.single_cursor();
        }
        // Selected with the mouse (or ⌘A): Visual, from there.
        if self.vim.mode == Mode::Normal && !self.selection.is_empty() {
            let range = self.selection.range();
            let forward = self.selection.head >= self.selection.anchor;
            self.vim.mode = Mode::Visual;
            (self.vim.anchor, self.vim.head) =
                if forward { (range.start, range.end.saturating_sub(1)) } else { (range.end.saturating_sub(1), range.start) };
        }
        if let Some(pending) = self.vim.pending.take() {
            return self.vim_pending(pending, key, window, cx);
        }
        // A count: 3w, 2dd (0 alone goes to the line's start).
        if let Key::Char(c @ '0'..='9') = key
            && (c != '0' || self.vim.count.is_some())
        {
            let digit = c.to_digit(10).unwrap_or(0) as usize;
            self.vim.count = Some(self.vim.count.unwrap_or(0).saturating_mul(10).saturating_add(digit).min(100_000));
            return true;
        }
        if matches!(&key, Key::Named(k) if k == "escape") || matches!(key, Key::Ctrl('[' | 'c')) {
            self.vim.count = None;
            self.vim.operator = None;
            if self.vim.mode.visual() {
                self.vim_end_visual(cx);
            } else {
                self.close_completion(cx);
                cx.notify();
            }
            return true;
        }
        let count = self.vim.count.take();
        if self.vim.mode.visual() {
            return self.vim_visual(key, count, window, cx);
        }
        // An operator waiting: this key is its motion, or the operator again (whole lines).
        if let Some((operator, before)) = self.vim.operator.take() {
            let n = before.unwrap_or(1).max(1) * count.unwrap_or(1).max(1);
            let total = (before.is_some() || count.is_some()).then_some(n);
            if let Key::Char(c) = key
                && Operator::of(c) == Some(operator)
            {
                let line = self.buffer.point(self.selection.head).0;
                let to = (line + n - 1).min(self.vim_last_line());
                let motion = Motion { to: self.line_start(to), linewise: true, inclusive: false };
                self.vim_operate(operator, self.selection.head, motion, window, cx);
                return true;
            }
            if key == Key::Char('g') {
                self.vim.operator = Some((operator, total));
                self.vim.pending = Some('g');
                return true;
            }
            // cw changes to the word's end, as ce.
            let key = match (operator, &key) {
                (Operator::Change, Key::Char('w')) if !self.at_blank() => Key::Char('e'),
                (Operator::Change, Key::Char('W')) if !self.at_blank() => Key::Char('E'),
                _ => key,
            };
            if let Some(motion) = self.vim_motion(&key, total, true) {
                self.vim_operate(operator, self.selection.head, motion, window, cx);
            }
            return true;
        }
        if let Some(motion) = self.vim_motion(&key, count, false) {
            if matches!(key, Key::Ctrl('d' | 'u' | 'f' | 'b')) {
                let lines = self.buffer.point(motion.to).0 as f32 - self.buffer.point(self.selection.head).0 as f32;
                self.scroll.target_y = (self.scroll.target_y + lines * f32::from(self.line_height())).max(0.);
            }
            let goal = self.vim.goal;
            self.vim_place(motion.to, cx);
            self.vim.goal = goal;
            return true;
        }
        let n = count.unwrap_or(1).max(1);
        let head = self.selection.head;
        let (line, _) = self.buffer.point(head);
        match key {
            Key::Char(c) if Operator::of(c).is_some() => {
                self.vim.operator = Operator::of(c).map(|op| (op, count));
            }
            Key::Char(c @ ('g' | 'r')) => {
                self.vim.count = count;
                self.vim.pending = Some(c);
            }
            Key::Char('i') => self.vim_insert_at(head, cx),
            Key::Char('a') => {
                let at = if self.buffer.line_len(line) == 0 { head } else { head + 1 };
                self.vim_insert_at(at, cx)
            }
            Key::Char('I') => self.vim_insert_at(self.first_non_blank(line), cx),
            Key::Char('A') => self.vim_insert_at(self.line_end(line), cx),
            Key::Char('o') => {
                self.vim.mode = Mode::Insert;
                self.newline_below(&NewlineBelow, window, cx);
            }
            Key::Char('O') => {
                self.vim.mode = Mode::Insert;
                self.newline_above(&NewlineAbove, window, cx);
            }
            Key::Char('v') => self.vim_start_visual(Mode::Visual, cx),
            Key::Char('V') => self.vim_start_visual(Mode::VisualLine, cx),
            Key::Char('x') => {
                if self.buffer.line_len(line) > 0 {
                    let end = (head + n).min(self.line_end(line));
                    self.vim_operate(Operator::Delete, head, Motion { to: end, linewise: false, inclusive: false }, window, cx);
                }
            }
            Key::Char('X') => {
                let start = head.saturating_sub(n).max(self.line_start(line));
                if start < head {
                    self.vim_operate(Operator::Delete, head, Motion { to: start, linewise: false, inclusive: false }, window, cx);
                }
            }
            Key::Char('s') => {
                let end = (head + n).min(self.line_end(line));
                self.vim_operate(Operator::Change, head, Motion { to: end, linewise: false, inclusive: false }, window, cx);
            }
            Key::Char(c @ ('D' | 'C' | 'Y' | 'S')) => {
                let to_line = (line + n - 1).min(self.vim_last_line());
                let (operator, motion) = match c {
                    'D' => (Operator::Delete, Motion { to: self.line_end(to_line), linewise: false, inclusive: false }),
                    'C' => (Operator::Change, Motion { to: self.line_end(to_line), linewise: false, inclusive: false }),
                    'Y' => (Operator::Yank, Motion { to: self.line_start(to_line), linewise: true, inclusive: false }),
                    _ => (Operator::Change, Motion { to: self.line_start(to_line), linewise: true, inclusive: false }),
                };
                self.vim_operate(operator, head, motion, window, cx);
            }
            Key::Char('p') => self.vim_put(true, n, cx),
            Key::Char('P') => self.vim_put(false, n, cx),
            Key::Char('u') => {
                for _ in 0..n {
                    self.step_history(true, cx);
                }
                self.vim_after_history(cx);
            }
            Key::Ctrl('r') => {
                for _ in 0..n {
                    self.step_history(false, cx);
                }
                self.vim_after_history(cx);
            }
            Key::Char('J') => {
                let last = (line + n.max(2) - 1).min(self.vim_last_line());
                if last > line {
                    self.selection = Selection { anchor: self.line_start(line), head: self.line_start(last) };
                    self.join_lines(&JoinLines, window, cx);
                    let at = self.selection.head;
                    self.vim_place(at, cx);
                }
            }
            Key::Char('~') => {
                let end = (head + n).min(self.line_end(line));
                if end > head {
                    let toggled: String = self.buffer.slice(head..end).chars().map(toggle_case).collect();
                    self.edit(head..end, &toggled, EditKind::Other, cx);
                    self.vim_place(end, cx);
                }
            }
            // Keys of Vim's not here yet, and other characters: nothing typed.
            Key::Char(_) => {}
            // Others (Tab, ⌃G): as Null has them.
            _ => return false,
        }
        true
    }

    fn at_blank(&self) -> bool {
        self.buffer.char_at(self.selection.head).is_none_or(char::is_whitespace)
    }

    /// The key after `g` or `r`.
    fn vim_pending(&mut self, pending: char, key: Key, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let count = self.vim.count.take();
        match (pending, key) {
            ('g', Key::Char('g')) => {
                let last = self.vim_last_line();
                let to = count.map_or(0, |c| c.saturating_sub(1).min(last));
                let motion = Motion { to: self.first_non_blank(to), linewise: true, inclusive: false };
                if let Some((operator, _)) = self.vim.operator.take() {
                    self.vim_operate(operator, self.selection.head, motion, window, cx);
                } else if self.vim.mode.visual() {
                    self.vim.head = motion.to;
                    self.vim_show_visual(cx);
                } else {
                    self.vim_place(motion.to, cx);
                }
            }
            ('r', Key::Char(c)) if !self.vim.mode.visual() => {
                let n = count.unwrap_or(1).max(1);
                let head = self.selection.head;
                let line = self.buffer.point(head).0;
                if head + n <= self.line_end(line) {
                    self.edit(head..head + n, &c.to_string().repeat(n), EditKind::Other, cx);
                    self.vim_place(head + n - 1, cx);
                }
            }
            _ => {
                self.vim.operator = None;
            }
        }
        true
    }

    fn vim_insert_at(&mut self, at: usize, cx: &mut Context<Self>) {
        self.vim.mode = Mode::Insert;
        self.selection = Selection::caret(at);
        self.goal_column = None;
        self.touch(cx);
    }

    fn vim_after_history(&mut self, cx: &mut Context<Self>) {
        self.single_cursor();
        let at = self.selection.range().start;
        self.vim.mode = Mode::Normal;
        self.vim_place(at, cx);
    }

    /// The text of `range`, kept to put back (and on the clipboard, as Vim with
    /// `clipboard=unnamed` has it).
    fn vim_yank(&mut self, text: String, linewise: bool, cx: &mut Context<Self>) {
        #[cfg(not(test))]
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
        #[cfg(test)]
        let _ = cx;
        self.vim.register = Some(Register { text, linewise });
    }

    /// What p puts: the clipboard when something else was copied since, else the register.
    fn vim_register(&self, cx: &App) -> Option<Register> {
        #[cfg(not(test))]
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
            && self.vim.register.as_ref().is_none_or(|r| r.text != text)
        {
            let linewise = text.ends_with('\n');
            return Some(Register { text, linewise });
        }
        #[cfg(test)]
        let _ = cx;
        self.vim.register.clone()
    }

    /// Line breaks as the file has them.
    fn vim_breaks(&self, text: &str) -> String {
        match self.style.line_ending {
            crate::file_style::LineEnding::Crlf => text.replace("\r\n", "\n").replace('\n', "\r\n"),
            crate::file_style::LineEnding::Lf => text.to_string(),
        }
    }

    /// `operator` from `from` to where `motion` goes.
    fn vim_operate(&mut self, operator: Operator, from: usize, motion: Motion, window: &mut Window, cx: &mut Context<Self>) {
        let (a, b) = if motion.to < from { (motion.to, from) } else { (from, motion.to) };
        if motion.linewise {
            let (first, last) = (self.buffer.point(a).0, self.buffer.point(b).0);
            return self.vim_operate_lines(operator, first, last, window, cx);
        }
        let end = if motion.inclusive { (b + 1).min(self.buffer.len_chars()) } else { b };
        let range = a..end;
        match operator {
            Operator::Delete | Operator::Change => {
                let text = self.buffer.slice(range.clone());
                if !text.is_empty() {
                    self.vim_yank(text, false, cx);
                    self.edit(range.clone(), "", EditKind::Other, cx);
                }
                if operator == Operator::Change {
                    self.vim_insert_at(range.start, cx);
                } else {
                    self.vim_place(range.start, cx);
                }
            }
            Operator::Yank => {
                let text = self.buffer.slice(range.clone());
                self.vim_yank(text, false, cx);
                self.vim_place(range.start, cx);
            }
            Operator::Indent | Operator::Outdent => {
                let (first, last) = (self.buffer.point(a).0, self.buffer.point(b).0);
                self.vim_operate_lines(operator, first, last, window, cx);
            }
        }
    }

    /// `operator` on whole lines `first..=last`.
    fn vim_operate_lines(
        &mut self,
        operator: Operator,
        first: usize,
        last: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let last = last.min(self.vim_last_line());
        let start = self.line_start(first);
        let after = self.line_start(last + 1);
        let text: String = (first..=last).map(|l| self.buffer.line_text(l) + "\n").collect();
        match operator {
            Operator::Yank => {
                self.vim_yank(text, true, cx);
                let at = if self.buffer.point(self.selection.head).0 > first { start } else { self.selection.head };
                self.vim_place(at.max(start), cx);
            }
            Operator::Delete => {
                self.vim_yank(text, true, cx);
                // The last lines: the break before them goes with them.
                let range = if after >= self.buffer.len_chars() && after == self.line_end(last) && first > 0 {
                    self.line_end(first - 1)..after
                } else {
                    start..after
                };
                self.edit(range, "", EditKind::Other, cx);
                let line = first.min(self.vim_last_line());
                let at = self.first_non_blank(line);
                self.vim_place(at, cx);
            }
            Operator::Change => {
                self.vim_yank(text, true, cx);
                // Its indentation stays, the text goes.
                let indent = self.first_non_blank(first) - start;
                let range = start + indent..self.line_end(last);
                self.edit(range, "", EditKind::Other, cx);
                let at = start + indent;
                self.vim_insert_at(at, cx);
            }
            Operator::Indent | Operator::Outdent => {
                self.selection = Selection { anchor: start, head: self.line_end(last) };
                if operator == Operator::Indent {
                    self.indent(&Indent, window, cx);
                } else {
                    self.outdent(&Outdent, window, cx);
                }
                let at = self.first_non_blank(first);
                self.vim_place(at, cx);
            }
        }
    }

    /// p (after the caret, or below its line) and P (before, or above), `count` times.
    fn vim_put(&mut self, after: bool, count: usize, cx: &mut Context<Self>) {
        let Some(register) = self.vim_register(cx) else { return };
        let head = self.selection.head;
        let line = self.buffer.point(head).0;
        if register.linewise {
            let body = register.text.strip_suffix('\n').unwrap_or(&register.text).to_string();
            let text = self.vim_breaks(&(body + "\n").repeat(count));
            let has_break = self.line_start(line + 1) > self.line_end(line);
            let (at, text, first) = if !after {
                (self.line_start(line), text, line)
            } else if has_break {
                (self.line_start(line + 1), text, line + 1)
            } else {
                // Below a last line with no break after it: the break goes before.
                let ending = self.style.line_ending.text();
                let text = format!("{ending}{}", text.strip_suffix(ending).unwrap_or(&text));
                (self.line_end(line), text, line + 1)
            };
            self.edit(at..at, &text, EditKind::Other, cx);
            let to = self.first_non_blank(first.min(self.vim_last_line()));
            return self.vim_place(to, cx);
        }
        let text = self.vim_breaks(&register.text.repeat(count));
        let at = if after && self.buffer.line_len(line) > 0 { (head + 1).min(self.line_end(line)) } else { head };
        self.edit(at..at, &text, EditKind::Other, cx);
        let end = at + text.chars().count();
        self.vim_place(end.saturating_sub(1).max(at), cx);
    }

    fn vim_start_visual(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.vim.mode = mode;
        self.vim.anchor = self.selection.head;
        self.vim.head = self.selection.head;
        self.vim_show_visual(cx);
    }

    /// The selection for Visual mode: from the anchor through the character at the head
    /// (whole lines in Visual Line).
    fn vim_show_visual(&mut self, cx: &mut Context<Self>) {
        let len = self.buffer.len_chars();
        let (anchor, head) = (self.vim.anchor.min(len), self.vim.head.min(len));
        self.selection = if self.vim.mode == Mode::VisualLine {
            let (a, h) = (self.buffer.point(anchor).0, self.buffer.point(head).0);
            if h >= a {
                Selection { anchor: self.line_start(a), head: self.line_start(h + 1).max(self.line_end(h)) }
            } else {
                Selection { anchor: self.line_start(a + 1).max(self.line_end(a)), head: self.line_start(h) }
            }
        } else if head >= anchor {
            Selection { anchor, head: (head + 1).min(len) }
        } else {
            Selection { anchor: (anchor + 1).min(len), head }
        };
        self.goal_column = None;
        self.touch(cx);
    }

    fn vim_end_visual(&mut self, cx: &mut Context<Self>) {
        self.vim.mode = Mode::Normal;
        let at = self.vim.head;
        self.vim_place(at, cx);
    }

    /// A key in Visual mode: a motion moves the head; an operator acts on the selection.
    fn vim_visual(&mut self, key: Key, count: Option<usize>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if key == Key::Char('g') {
            self.vim.count = count;
            self.vim.pending = Some('g');
            return true;
        }
        if let Some(motion) = self.vim_motion(&key, count, false) {
            let goal = self.vim.goal;
            self.vim.head = self.vim_clamp(motion.to);
            self.vim.goal = goal;
            self.vim_show_visual(cx);
            return true;
        }
        let (a, b) = (self.vim.anchor.min(self.vim.head), self.vim.anchor.max(self.vim.head));
        let lines = self.vim.mode == Mode::VisualLine;
        let operator = match key {
            Key::Char('d' | 'x') => Some(Operator::Delete),
            Key::Char('c' | 's') => Some(Operator::Change),
            Key::Char('y') => Some(Operator::Yank),
            Key::Char('>') => Some(Operator::Indent),
            Key::Char('<') => Some(Operator::Outdent),
            _ => None,
        };
        if let Some(operator) = operator {
            self.vim.mode = Mode::Normal;
            let motion = Motion { to: b, linewise: lines, inclusive: true };
            self.selection = Selection::caret(a);
            self.vim_operate(operator, a, motion, window, cx);
            return true;
        }
        match key {
            Key::Char('D' | 'X' | 'Y' | 'C' | 'S' | 'R') => {
                let operator = match key {
                    Key::Char('Y') => Operator::Yank,
                    Key::Char('C' | 'S' | 'R') => Operator::Change,
                    _ => Operator::Delete,
                };
                self.vim.mode = Mode::Normal;
                let (first, last) = (self.buffer.point(a).0, self.buffer.point(b).0);
                self.selection = Selection::caret(a);
                self.vim_operate_lines(operator, first, last, window, cx);
            }
            Key::Char('v') if self.vim.mode == Mode::Visual => self.vim_end_visual(cx),
            Key::Char('V') if lines => self.vim_end_visual(cx),
            Key::Char('v') => {
                self.vim.mode = Mode::Visual;
                self.vim_show_visual(cx);
            }
            Key::Char('V') => {
                self.vim.mode = Mode::VisualLine;
                self.vim_show_visual(cx);
            }
            Key::Char('o') => {
                std::mem::swap(&mut self.vim.anchor, &mut self.vim.head);
                self.vim_show_visual(cx);
            }
            Key::Char('~' | 'u' | 'U') => {
                let range = self.selection.range();
                let text = self.buffer.slice(range.clone());
                let changed: String = match key {
                    Key::Char('u') => text.to_lowercase(),
                    Key::Char('U') => text.to_uppercase(),
                    _ => text.chars().map(toggle_case).collect(),
                };
                self.edit(range.clone(), &changed, EditKind::Other, cx);
                self.vim.mode = Mode::Normal;
                self.vim_place(range.start, cx);
            }
            Key::Char('J') => {
                let (first, last) = (self.buffer.point(a).0, self.buffer.point(b).0.max(self.buffer.point(a).0 + 1));
                self.vim.mode = Mode::Normal;
                self.selection = Selection { anchor: self.line_start(first), head: self.line_start(last.min(self.vim_last_line())) };
                self.join_lines(&JoinLines, window, cx);
                let at = self.selection.head;
                self.vim_place(at, cx);
            }
            Key::Char('p' | 'P') => {
                let Some(register) = self.vim_register(cx) else { return true };
                let range = self.selection.range();
                let replaced = self.buffer.slice(range.clone());
                let text = self.vim_breaks(&register.text);
                self.edit(range.clone(), &text, EditKind::Other, cx);
                // What was there is what p puts next (as Vim's unnamed register has it).
                if key == Key::Char('p') {
                    self.vim_yank(replaced, lines, cx);
                }
                self.vim.mode = Mode::Normal;
                self.vim_place(range.start, cx);
            }
            Key::Char(_) => {}
            _ => return false,
        }
        true
    }
}

fn toggle_case(c: char) -> char {
    if c.is_lowercase() {
        c.to_uppercase().next().unwrap_or(c)
    } else if c.is_uppercase() {
        c.to_lowercase().next().unwrap_or(c)
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    /// An editor with Vim's keys, the caret at the `|` in `text` (taken out).
    fn vim<'a>(text: &str, cx: &'a mut TestAppContext) -> (Entity<Editor>, &'a mut gpui::VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(Settings { keymap: crate::keymap::Keymap::Vim, ..Settings::default() });
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Vim, cx);
        });
        let at = text.find('|').unwrap();
        let text = text.replace('|', "");
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(Buffer::from_text(&text), Some(PathBuf::from("x.txt")), cx));
        e.update_in(cx, |e, window, _| {
            window.focus(&e.focus_handle);
            e.selection = Selection::caret(at);
        });
        (e, cx)
    }

    /// The text with `|` where the caret is.
    fn shown(e: &Entity<Editor>, cx: &mut gpui::VisualTestContext) -> String {
        e.read_with(cx, |e, _| {
            let mut text = e.buffer.to_string();
            text.insert(text.char_indices().nth(e.shown_caret()).map_or(text.len(), |(i, _)| i), '|');
            text
        })
    }

    #[gpui::test]
    fn moves_by_words_lines_and_ends(cx: &mut TestAppContext) {
        let (e, cx) = vim("|let total = price(qty);\n  next line\n\nend\n", cx);
        cx.simulate_keystrokes("w");
        assert_eq!(shown(&e, cx), "let |total = price(qty);\n  next line\n\nend\n");
        cx.simulate_keystrokes("3 w");
        assert_eq!(shown(&e, cx), "let total = price|(qty);\n  next line\n\nend\n");
        cx.simulate_keystrokes("e");
        assert_eq!(shown(&e, cx), "let total = price(qt|y);\n  next line\n\nend\n");
        cx.simulate_keystrokes("b b");
        assert_eq!(shown(&e, cx), "let total = price|(qty);\n  next line\n\nend\n");
        cx.simulate_keystrokes("$");
        assert_eq!(shown(&e, cx), "let total = price(qty)|;\n  next line\n\nend\n");
        cx.simulate_keystrokes("j");
        assert_eq!(shown(&e, cx), "let total = price(qty);\n  next lin|e\n\nend\n", "$ keeps to the line's end");
        cx.simulate_keystrokes("^");
        assert_eq!(shown(&e, cx), "let total = price(qty);\n  |next line\n\nend\n");
        cx.simulate_keystrokes("0 j");
        assert_eq!(shown(&e, cx), "let total = price(qty);\n  next line\n|\nend\n", "an empty line");
        cx.simulate_keystrokes("G");
        assert_eq!(shown(&e, cx), "let total = price(qty);\n  next line\n\n|end\n", "the last line with text");
        cx.simulate_keystrokes("g g l l");
        assert_eq!(shown(&e, cx), "le|t total = price(qty);\n  next line\n\nend\n");
        cx.simulate_keystrokes("2 G");
        assert_eq!(shown(&e, cx), "let total = price(qty);\n  |next line\n\nend\n");
        // Typing letters with no meaning types nothing.
        cx.simulate_keystrokes("z");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "let total = price(qty);\n  next line\n\nend\n");
    }

    #[gpui::test]
    fn operators_take_motions_and_counts(cx: &mut TestAppContext) {
        let (e, cx) = vim("|one two three\nfour\nfive\nsix\n", cx);
        cx.simulate_keystrokes("d w");
        assert_eq!(shown(&e, cx), "|two three\nfour\nfive\nsix\n");
        cx.simulate_keystrokes("c w T W O escape");
        assert_eq!(shown(&e, cx), "TW|O three\nfour\nfive\nsix\n", "cw changes the word, not the space after");
        cx.simulate_keystrokes("j d d");
        assert_eq!(shown(&e, cx), "TWO three\n|five\nsix\n");
        cx.simulate_keystrokes("u");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "TWO three\nfour\nfive\nsix\n");
        cx.simulate_keystrokes("ctrl-r");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "TWO three\nfive\nsix\n");
        // The last lines take the break before them.
        cx.simulate_keystrokes("2 d d");
        assert_eq!(shown(&e, cx), "|TWO three\n");
        cx.simulate_keystrokes("w D");
        assert_eq!(shown(&e, cx), "TWO| \n");
        cx.simulate_keystrokes("x");
        assert_eq!(shown(&e, cx), "TW|O\n", "x at the end stays on the line");
        cx.simulate_keystrokes("o n e x t escape d d");
        assert_eq!(shown(&e, cx), "|TWO\n");
    }

    #[gpui::test]
    fn yank_and_put_lines_and_words(cx: &mut TestAppContext) {
        let (e, cx) = vim("|alpha beta\ngamma\n", cx);
        cx.simulate_keystrokes("y y p");
        assert_eq!(shown(&e, cx), "alpha beta\n|alpha beta\ngamma\n");
        cx.simulate_keystrokes("y w j P");
        assert_eq!(shown(&e, cx), "alpha beta\nalpha beta\nalpha| gamma\n");
        cx.simulate_keystrokes("G d d k P");
        assert_eq!(shown(&e, cx), "|alpha gamma\nalpha beta\nalpha beta\n");
    }

    #[gpui::test]
    fn put_below_a_last_line_with_no_break(cx: &mut TestAppContext) {
        let (e, cx) = vim("|one\ntwo", cx);
        cx.simulate_keystrokes("y y G p");
        assert_eq!(shown(&e, cx), "one\ntwo\n|one");
        // Taken away again: with the break before it, as there's none after.
        cx.simulate_keystrokes("d d");
        assert_eq!(shown(&e, cx), "one\n|two");
    }

    #[gpui::test]
    fn insert_modes_and_escape(cx: &mut TestAppContext) {
        let (e, cx) = vim("  fn |main() {}\n", cx);
        cx.simulate_keystrokes("A x escape");
        assert_eq!(shown(&e, cx), "  fn main() {}|x\n", "Esc steps back onto what was typed");
        cx.simulate_keystrokes("I y escape");
        assert_eq!(shown(&e, cx), "  |yfn main() {}x\n");
        cx.simulate_keystrokes("o z escape");
        assert_eq!(shown(&e, cx), "  yfn main() {}x\n  |z\n", "o keeps the indentation");
        e.read_with(cx, |e, cx| assert_eq!(e.vim_mode(cx), Some(Mode::Normal)));
        cx.simulate_keystrokes("i");
        e.read_with(cx, |e, cx| {
            assert_eq!(e.vim_mode(cx), Some(Mode::Insert));
            assert!(!e.block_caret(cx));
        });
    }

    #[gpui::test]
    fn visual_mode_selects_and_acts(cx: &mut TestAppContext) {
        let (e, cx) = vim("|one two\nthree\nfour\n", cx);
        cx.simulate_keystrokes("v e");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.slice(e.selection.range())), "one");
        assert_eq!(shown(&e, cx), "on|e two\nthree\nfour\n", "the block on the last selected character");
        cx.simulate_keystrokes("U");
        assert_eq!(shown(&e, cx), "|ONE two\nthree\nfour\n");
        cx.simulate_keystrokes("j V j d");
        assert_eq!(shown(&e, cx), "|ONE two\n");
        cx.simulate_keystrokes("k v $ y");
        e.read_with(cx, |e, cx| assert_eq!(e.vim_mode(cx), Some(Mode::Normal)));
        assert_eq!(e.read_with(cx, |e, _| e.vim.register.clone().unwrap().text), "ONE two");
        cx.simulate_keystrokes("v escape");
        assert!(e.read_with(cx, |e, _| e.selection.is_empty()));
    }

    #[gpui::test]
    fn command_keys_stay_null_s(cx: &mut TestAppContext) {
        let (e, cx) = vim("|abc abc\n", cx);
        // ⌘D: Null's next match, in Normal mode too.
        cx.simulate_keystrokes("cmd-d");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.slice(e.selection.range())), "abc");
        cx.simulate_keystrokes("escape");
        // The keymap off: letters type again.
        cx.update(|_, cx| crate::settings::update(cx, |s| s.keymap = crate::keymap::Keymap::Null));
        cx.simulate_keystrokes("x");
        assert!(e.read_with(cx, |e, _| e.buffer.to_string()).contains('x'));
    }
}
