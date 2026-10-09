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
    VisualBlock,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "Normal",
            Mode::Insert => "Insert",
            Mode::Visual => "Visual",
            Mode::VisualLine => "Visual Line",
            Mode::VisualBlock => "Visual Block",
        }
    }

    fn visual(self) -> bool {
        matches!(self, Mode::Visual | Mode::VisualLine | Mode::VisualBlock)
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
#[derive(Clone, Debug, Default)]
struct Register {
    text: String,
    linewise: bool,
    /// A Visual Block's columns, a line each: put back as a block.
    block: bool,
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
    /// Visual mode: where it started, and the end that moves.
    anchor: usize,
    head: usize,
    /// The column `j` and `k` keep to (`usize::MAX` after `$`: the line's end).
    goal: Option<usize>,
    /// The last f F t T, and its character, for ; and ,.
    last_find: Option<(char, char)>,
    /// The keys of the command being typed, and the text's revision before it.
    typed: Vec<Key>,
    change_from: u64,
    /// The last change, for `.`: its keys, and what was typed after them (Insert mode).
    last_change: Vec<Key>,
    last_insert: Option<String>,
    /// Where typing began after a change, to know what was typed.
    insert_from: Option<usize>,
    /// Replaying the last change (`.`): not a change of its own.
    replaying: bool,
    /// The search went backwards (?, #): n goes on backwards.
    search_back: bool,
    /// Where the caret was when / or ? opened the find bar (Esc goes back there).
    before_search: Option<usize>,
    /// The undo history's length when the change being made began: all it adds is one
    /// step (the typing after `cw` included), as Vim undoes a change.
    undo_from: Option<usize>,
    /// The selection Visual mode last showed, and the text's revision then: changed
    /// since (a click, ⌘X, an undo), Vim takes the selection as it is now.
    shown: Option<(Selection, u64)>,
    /// Where Vim last put the caret: moved since by other means, `goal` is forgotten.
    placed: usize,
    /// The keymap registration this state belongs to (switched off and on: a fresh one).
    epoch: u64,
    /// The register named for the next command ("a), and whether it was named just now.
    register_name: Option<char>,
    name_fresh: bool,
    /// ma to mz: line and column, and the text's revision then (they follow edits since).
    marks: std::collections::HashMap<char, (usize, usize, u64)>,
    /// Where the last jump (G, %, n, a mark) left from, for '' and ``.
    jump_back: Option<(usize, usize, u64)>,
    /// qa … q: the register recorded into, and the keys so far.
    recording: Option<(char, Vec<gpui::Keystroke>)>,
    /// A macro's keys still to play, one at a time (each after the last has done its work).
    queue: std::collections::VecDeque<gpui::Keystroke>,
    /// Macros played since the last key typed by hand (one that plays itself stops).
    macro_runs: usize,
    in_replay: bool,
    /// 3i, 2o: what's typed is typed this many more times (on new lines, for o and O).
    insert_repeat: Option<(usize, bool)>,
    /// Visual Block after $: every line to its end, however long.
    block_to_end: bool,
    /// How many times the last typing went in (3i: three), for `.`.
    last_insert_times: usize,
    /// The caret moved in Insert mode by other means than typing (an arrow, a click): what's
    /// between where typing began and the caret isn't all typed.
    insert_moved: bool,
}

/// Registers and macros: one set for every file, as Vim has them (marks are each file's).
#[derive(Default)]
struct Shared {
    /// The unnamed register: what was last yanked or deleted.
    unnamed: Option<Register>,
    /// "a to "z, and "0 (the last yank).
    registers: std::collections::HashMap<char, Register>,
    macros: std::collections::HashMap<char, Vec<gpui::Keystroke>>,
    last_macro: Option<char>,
}

impl gpui::Global for Shared {}

/// A key as Vim reads it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Key {
    Char(char),
    Ctrl(char),
    Named(String),
}

/// A macro's next key, played as if typed; then the one after, once this one's work (a find
/// bar opened, a command line) is done.
fn play_queued(editor: gpui::WeakEntity<Editor>, window: &mut Window, cx: &mut App) {
    let next = editor.update(cx, |e, _| {
        let next = e.vim.queue.pop_front();
        if next.is_none() {
            e.vim.in_replay = false;
        }
        next
    });
    let Ok(Some(key)) = next else { return };
    window.dispatch_keystroke(key, cx);
    window.defer(cx, move |window, cx| play_queued(editor, window, cx));
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
            .update(cx, |editor, cx| {
                if editor.focus_handle.is_focused(window) {
                    return editor.vim_key(&keystroke, window, cx);
                }
                // Recording, and typed in its find bar or : line: in the macro too.
                if !editor.vim.in_replay
                    && let Some((_, keys)) = &mut editor.vim.recording
                {
                    keys.push(keystroke.clone());
                }
                false
            })
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

    /// A click in the text: typing (in Insert mode) no longer goes on from where it began.
    pub(super) fn vim_clicked(&mut self) {
        if self.vim.mode == Mode::Insert {
            self.vim.insert_moved = true;
        }
    }

    /// The register a macro is being recorded into (qa).
    pub fn vim_recording(&self) -> Option<char> {
        self.vim.recording.as_ref().map(|(name, _)| *name)
    }

    /// Where the caret shows: in Visual mode, on the character at the moving end (the
    /// selection itself ends after it).
    pub fn shown_caret(&self, cx: &App) -> usize {
        if on(cx) && self.vim.mode.visual() && !self.selection.is_empty() {
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
        // The keymap switched off and on since: what was pending then is gone.
        let epoch = crate::keymap::epoch(cx);
        if self.vim.epoch != epoch {
            self.vim = Vim { epoch, ..Vim::default() };
        }
        if !self.vim.in_replay {
            self.vim.macro_runs = 0;
        }
        // Recording a macro: every key, typing included (not those a macro plays).
        if !self.vim.in_replay
            && let Some((_, keys)) = &mut self.vim.recording
        {
            keys.push(keystroke.clone());
        }
        if self.vim.mode == Mode::Insert {
            // Moved by other means than typing: what follows isn't all typed (3i, `.`).
            let moves = matches!(keystroke.key.as_str(), "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown");
            if moves {
                self.vim.insert_moved = true;
            }
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
                // Null's Emacs keys edit or move (⌃O opens a line, ⌃Y yanks): not here.
                Some(c) if keystroke.key.chars().count() == 1 && "aehknopty".contains(c) => return true,
                Some(c) if keystroke.key.chars().count() == 1 => Key::Ctrl(c),
                _ => return false,
            }
        } else if keystroke.key == "tab" {
            // Tab keeps an AI change or a suggestion, goes to a snippet's next place: Null's.
            return false;
        } else if keystroke.key == "enter" {
            Key::Named("enter".into())
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
        let mut head = self.selection.head;
        let moved = std::mem::take(&mut self.vim.insert_moved);
        self.vim.last_insert_times = 1;
        // What was typed after the change began, for `.` to type again (not when the caret
        // went elsewhere meanwhile: what's between isn't all typed).
        if let Some(from) = self.vim.insert_from.take()
            && from <= head
            && !moved
        {
            let typed = self.buffer.slice(from..head);
            // 3i, 2o: typed again, as many more times (within reason).
            if let Some((more, lines)) = self.vim.insert_repeat.take()
                && !typed.is_empty()
                && more.saturating_mul(typed.len()) <= 1 << 20
            {
                let text = if lines {
                    // The new line's indentation: what was there before typing began.
                    let (line, column) = self.buffer.point(from);
                    let indent: String = self.buffer.line_text(line).chars().take(column).collect();
                    format!("{}{indent}{typed}", self.style.line_ending.text()).repeat(more)
                } else {
                    typed.repeat(more)
                };
                let at = if lines { self.line_end(self.buffer.point(head).0) } else { head };
                self.edit(at..at, &text, EditKind::Other, cx);
                head = self.selection.head;
                self.vim.last_insert_times = more + 1;
            }
            self.vim.last_insert = Some(typed);
        } else if moved {
            self.vim.last_insert = None;
        }
        self.vim.insert_repeat = None;
        if !self.vim.replaying {
            self.vim_one_undo_step();
        }
        // The next typing (a, A right after) is a step of its own.
        self.last_edit = None;
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
        self.vim.placed = at;
        self.selection = Selection::caret(at);
        self.goal_column = None;
        self.touch(cx);
    }

    /// Where a jump leaves from, for '' and `` to come back to.
    fn vim_note_jump(&mut self) {
        let (line, column) = self.buffer.point(self.vim_at());
        self.vim.jump_back = Some((line, column, self.buffer.revision()));
    }

    /// Where a mark is now: its line moved by the edits since it was set.
    fn vim_mark_now(&self, (line, column, revision): (usize, usize, u64)) -> (usize, usize) {
        let mut line = line;
        if let Some(edits) = self.buffer.edits_since(revision) {
            for edit in edits {
                line = super::breakpoints::move_line(line, edit);
            }
        }
        (line, column)
    }

    /// @a: the keys recorded in "a, `count` times, as if typed (once this key is done).
    fn vim_play(&mut self, name: char, count: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(keys) = cx.try_global::<Shared>().and_then(|s| s.macros.get(&name)).cloned() else { return };
        cx.default_global::<Shared>().last_macro = Some(name);
        // One that plays itself would never end.
        self.vim.macro_runs += 1;
        if self.vim.macro_runs > 100 || keys.is_empty() {
            return;
        }
        let keys: Vec<gpui::Keystroke> = keys.iter().cloned().cycle().take(keys.len() * count.min(1000)).collect();
        // Played from another: here, before the rest of that one (as Vim stuffs its keys).
        for key in keys.into_iter().rev() {
            self.vim.queue.push_front(key);
        }
        if !self.vim.in_replay {
            self.vim.in_replay = true;
            let editor = cx.entity().downgrade();
            window.defer(cx, move |window, cx| play_queued(editor, window, cx));
        }
    }

    /// Stops recording: the keys so far, but the q that stopped it (and what came before it
    /// in the same command), are the macro.
    fn vim_stop_recording(&mut self, typed_before: usize, cx: &mut Context<Self>) {
        let Some((name, mut keys)) = self.vim.recording.take() else { return };
        keys.truncate(keys.len().saturating_sub(typed_before + 1));
        cx.default_global::<Shared>().macros.insert(name, keys);
        cx.notify();
    }

    /// Visual Block's lines, each with its part between the block's columns.
    fn vim_block_ranges(&self) -> Vec<(usize, Range<usize>)> {
        let len = self.buffer.len_chars();
        let (anchor, head) = (self.buffer.point(self.vim.anchor.min(len)), self.buffer.point(self.vim.head.min(len)));
        let (first, last) = (anchor.0.min(head.0), anchor.0.max(head.0));
        // Columns as shown (a tab is as wide as it's drawn), so the block is straight.
        let (a, h) = (self.display_column(anchor.0, anchor.1), self.display_column(head.0, head.1));
        let (left, right) = (a.min(h), a.max(h));
        (first..=last)
            .map(|line| {
                let start = self.line_start(line);
                let from = self.column_char(line, left).0;
                // $: every line to its end.
                let to = if self.vim.block_to_end { self.buffer.line_len(line) } else { self.column_char(line, right + 1).0 };
                (line, start + from..start + to.max(from))
            })
            .collect()
    }

    /// A key in Visual Block mode: d x y c s I A act on every line of the block.
    fn vim_block(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let ranges = self.vim_block_ranges();
        let top_left = ranges.first().map_or(0, |(_, r)| r.start);
        let left = self.buffer.point(top_left).1;
        let text: String = ranges.iter().map(|(_, r)| self.buffer.slice(r.clone())).collect::<Vec<_>>().join("\n");
        let carets = |this: &mut Self, at: &dyn Fn(&Range<usize>) -> Option<usize>| {
            let cursors: Vec<(Cursor, bool)> = ranges
                .iter()
                .filter_map(|(_, r)| at(r))
                .enumerate()
                .map(|(i, at)| (Cursor::new(Selection::caret(at)), i == 0))
                .collect();
            if !cursors.is_empty() {
                this.set_cursors(cursors);
            }
        };
        match key {
            Key::Char('d' | 'x' | 'y' | 'c' | 's') => {
                let yank = key == Key::Char('y');
                self.vim_keep(Register { text, linewise: false, block: true }, yank, cx);
                if !yank {
                    let edits: Vec<(Range<usize>, String)> =
                        ranges.iter().filter(|(_, r)| !r.is_empty()).map(|(_, r)| (r.clone(), String::new())).collect();
                    self.apply_char_edits(edits, cx);
                }
                self.single_cursor();
                if matches!(key, Key::Char('c' | 's')) {
                    // Typed on every line of the block (that reaches its columns).
                    self.vim.mode = Mode::Insert;
                    let lines: Vec<usize> = ranges.iter().map(|(l, _)| *l).collect();
                    let cursors: Vec<(Cursor, bool)> = lines
                        .iter()
                        .filter(|l| self.buffer.line_len(**l) > left)
                        .enumerate()
                        .map(|(i, l)| (Cursor::new(Selection::caret(self.line_start(*l) + left)), i == 0))
                        .collect();
                    if !cursors.is_empty() {
                        self.set_cursors(cursors);
                    }
                    self.touch(cx);
                } else {
                    self.vim.mode = Mode::Normal;
                    self.vim_place(top_left, cx);
                }
            }
            Key::Char('I') => {
                self.vim.mode = Mode::Insert;
                // Lines that reach the block: shorter ones are left alone.
                let left_of = |r: &Range<usize>| (!r.is_empty()).then_some(r.start);
                carets(self, &left_of);
                self.touch(cx);
            }
            Key::Char('A') => {
                self.vim.mode = Mode::Insert;
                let after = |r: &Range<usize>| (!r.is_empty()).then_some(r.end);
                carets(self, &after);
                self.touch(cx);
            }
            Key::Char('o') => {
                std::mem::swap(&mut self.vim.anchor, &mut self.vim.head);
                self.vim_show_visual(cx);
            }
            Key::Char('v') => {
                self.vim.mode = Mode::Visual;
                self.single_cursor();
                self.vim_show_visual(cx);
            }
            Key::Char('V') => {
                self.vim.mode = Mode::VisualLine;
                self.single_cursor();
                self.vim_show_visual(cx);
            }
            Key::Ctrl('v') => self.vim_end_visual(cx),
            Key::Char(_) => {}
            _ => return false,
        }
        let _ = window;
        true
    }

    /// Where Vim acts from: Visual mode's moving end, else the caret (never past the text).
    fn vim_at(&self) -> usize {
        let at = if self.vim.mode.visual() { self.vim.head } else { self.selection.head };
        at.min(self.buffer.len_chars())
    }

    /// The column `char_column` of `line` is shown at, tabs taking the room they're drawn in.
    fn display_column(&self, line: usize, char_column: usize) -> usize {
        let tab = self.style.indent.width().max(1);
        self.buffer.line_text(line).chars().take(char_column).fold(0, |col, c| {
            if c == '\t' { (col / tab + 1) * tab } else { col + 1 }
        })
    }

    /// The character of `line` at shown column `column`, and how many spaces short of it
    /// the line ends (a short line).
    fn column_char(&self, line: usize, column: usize) -> (usize, usize) {
        let tab = self.style.indent.width().max(1);
        let mut shown = 0;
        for (i, c) in self.buffer.line_text(line).chars().enumerate() {
            let next = if c == '\t' { (shown / tab + 1) * tab } else { shown + 1 };
            if next > column {
                return (i, 0);
            }
            shown = next;
        }
        (self.buffer.line_len(line), column - shown)
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
        let at = self.vim_at();
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
            Key::Char(c @ (';' | ',')) => {
                let (kind, ch) = self.vim.last_find?;
                let kind = if *c == ';' {
                    kind
                } else {
                    match kind {
                        'f' => 'F',
                        'F' => 'f',
                        't' => 'T',
                        _ => 't',
                    }
                };
                return self.vim_find(kind, ch, n, true);
            }
            // 50%: half way down the file.
            Key::Char('%') if count.is_some() => {
                let to = (n.min(100) * (last + 1)).div_ceil(100).saturating_sub(1);
                Motion { to: self.first_non_blank(to), linewise: true, inclusive: false }
            }
            Key::Char('%') => inclusive(self.vim_matching_bracket(at)?),
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

    /// One key in Normal or Visual mode. True when Vim took it. The keys of a command that
    /// changed the text are kept, for `.`.
    pub(super) fn vim_input(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.vim.replaying {
            return self.vim_step(key, window, cx);
        }
        if self.vim.typed.is_empty() {
            self.vim.change_from = self.buffer.revision();
            self.vim.undo_from = Some(self.undo_stack.len());
        }
        let normal = self.vim.mode == Mode::Normal && self.selection.is_empty();
        if normal {
            self.vim.typed.push(key.clone());
        }
        // A new command: no count from an insert before it, nothing moved yet.
        if normal && self.vim.typed.len() == 1 {
            self.vim.insert_repeat = None;
            self.vim.insert_moved = false;
        }
        self.vim.name_fresh = false;
        let taken = self.vim_step(key, window, cx);
        let waiting = self.vim.count.is_some() || self.vim.operator.is_some() || self.vim.pending.is_some();
        if !waiting {
            // A register named ("a): for the command after it, then no more.
            if !self.vim.name_fresh {
                self.vim.register_name = None;
            }
            let typed = std::mem::take(&mut self.vim.typed);
            // Undo, redo and . itself (the command's key, not a character it took: ct. is a change).
            let command = typed.iter().find(|k| !matches!(k, Key::Char('0'..='9')));
            let history = matches!(command, Some(Key::Char('u' | '.') | Key::Ctrl('r')));
            let changed = self.buffer.revision() != self.vim.change_from || self.vim.mode == Mode::Insert;
            if normal && changed && !history && !typed.is_empty() {
                self.vim.last_change = typed;
                self.vim.last_insert = None;
                self.vim.insert_from = (self.vim.mode == Mode::Insert).then_some(self.selection.head);
                if self.vim.mode != Mode::Insert {
                    self.vim_one_undo_step();
                }
            }
        }
        taken
    }

    fn vim_step(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) -> bool {
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
        if !self.extra.is_empty() && self.vim.mode != Mode::VisualBlock {
            self.single_cursor();
        }
        // Visual's selection changed by other means (a click, ⌘X, an undo): taken as it is.
        if self.vim.mode.visual() && self.vim.shown != Some((self.selection, self.buffer.revision())) {
            if self.vim.mode == Mode::VisualBlock {
                self.single_cursor();
            }
            self.vim.mode = Mode::Normal;
            self.vim.operator = None;
            self.vim.count = None;
        }
        // The caret moved by other means: j and k forget their column; on the line's end
        // (a click after it), back on its last character.
        if self.vim.mode == Mode::Normal && self.selection.is_empty() && self.selection.head != self.vim.placed {
            self.vim.goal = None;
            let at = self.vim_clamp(self.selection.head.min(self.buffer.len_chars()));
            self.selection = Selection::caret(at);
            self.vim.placed = at;
        }
        // Selected with the mouse (or ⌘A): Visual, from there (an operator waiting is dropped).
        if self.vim.mode == Mode::Normal && !self.selection.is_empty() {
            self.vim.operator = None;
            self.vim.count = None;
            let range = self.selection.range();
            let forward = self.selection.head >= self.selection.anchor;
            self.vim.mode = Mode::Visual;
            (self.vim.anchor, self.vim.head) =
                if forward { (range.start, range.end.saturating_sub(1)) } else { (range.end.saturating_sub(1), range.start) };
            self.vim.shown = Some((self.selection, self.buffer.revision()));
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
            // Nothing of Vim's to cancel: Null's Escape (an AI change taken back, a note
            // closed, search marks cleared).
            let nothing = self.vim.count.is_none() && self.vim.operator.is_none() && !self.vim.mode.visual();
            if nothing && key == Key::Named("escape".into()) {
                return false;
            }
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
        if self.vim.mode.visual() && key == Key::Char('q') && self.vim.recording.is_some() {
            self.vim_stop_recording(0, cx);
            return true;
        }
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
            // g (gg), f t F T (to a character), i a (a text object): one more key.
            if let Key::Char(c @ ('g' | 'f' | 'F' | 't' | 'T' | 'i' | 'a' | '\'' | '`')) = key {
                self.vim.operator = Some((operator, total));
                self.vim.pending = Some(c);
                return true;
            }
            // cw changes to the word's end (this word's, when on its last character).
            if operator == Operator::Change
                && let Key::Char(c @ ('w' | 'W')) = key
                && !self.at_blank()
            {
                let big = c == 'W';
                let rope = self.buffer.rope();
                let mut to = self.selection.head;
                for i in 0..n {
                    let more = rope.get_char(to + 1).is_some_and(|next| class(next, big) == class(rope.char(to), big));
                    to = if i == 0 && !more { to } else { self.word_end(to, big) };
                }
                let motion = Motion { to, linewise: false, inclusive: true };
                self.vim_operate(operator, self.selection.head, motion, window, cx);
                return true;
            }
            if let Some(motion) = self.vim_motion(&key, total, true) {
                self.vim_operate(operator, self.selection.head, motion, window, cx);
            }
            return true;
        }
        if matches!(key, Key::Char('G' | '%')) {
            self.vim_note_jump();
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
            Key::Char(c @ ('g' | 'r' | 'f' | 'F' | 't' | 'T' | 'm' | '\'' | '`' | '"' | '@')) => {
                self.vim.count = count;
                self.vim.pending = Some(c);
            }
            // q: stops a recording; else starts one (into the register after it).
            Key::Char('q') if self.vim.recording.is_some() => {
                let before = self.vim.typed.len().saturating_sub(1);
                self.vim_stop_recording(before, cx);
            }
            Key::Char('q') => self.vim.pending = Some('q'),
            Key::Ctrl('v') => self.vim_start_visual(Mode::VisualBlock, cx),
            Key::Char('.') => self.vim_repeat(count, window, cx),
            Key::Char(c @ ('/' | '?')) => {
                self.vim_note_jump();
                self.vim.search_back = c == '?';
                self.vim.before_search = Some(head);
                // Found from after the caret: the match it's on isn't the one looked for.
                self.selection = Selection::caret((head + 1).min(self.buffer.len_chars()));
                self.deploy_find_bar(false, window, cx);
            }
            Key::Char(c @ ('n' | 'N')) => {
                self.vim_note_jump();
                for _ in 0..n {
                    self.vim_next_match((c == 'n') != self.vim.search_back, cx);
                }
            }
            Key::Char(c @ ('*' | '#')) => {
                self.vim_note_jump();
                self.vim_search_word(c == '#', n, cx)
            }
            Key::Char(':') => cx.emit(EditorEvent::VimCommandLine),
            Key::Char(c @ ('i' | 'a' | 'I' | 'A' | 'o' | 'O')) => {
                // 3ifoo: typed three times; 2o: on two new lines.
                self.vim.insert_repeat = (n > 1).then_some((n - 1, matches!(c, 'o' | 'O')));
                match c {
                    'i' => self.vim_insert_at(head, cx),
                    'a' => {
                        let at =
                            if self.buffer.line_len(line) == 0 { head } else { (head + 1).min(self.line_end(line)) };
                        self.vim_insert_at(at, cx)
                    }
                    'I' => self.vim_insert_at(self.first_non_blank(line), cx),
                    'A' => self.vim_insert_at(self.line_end(line), cx),
                    'o' => {
                        self.vim.mode = Mode::Insert;
                        self.newline_below(&NewlineBelow, window, cx);
                    }
                    _ => {
                        self.vim.mode = Mode::Insert;
                        self.newline_above(&NewlineAbove, window, cx);
                    }
                }
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
                    self.selection = Selection { anchor: self.line_start(line), head: self.line_end(last) };
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

    /// The key after `g`, `r`, f F t T, or i a (a text object).
    fn vim_pending(&mut self, pending: char, key: Key, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let operator = self.vim.operator.take();
        let count = self.vim.count.take().or(operator.and_then(|(_, n)| n));
        let operator = operator.map(|(op, _)| op);
        let n = count.unwrap_or(1).max(1);
        match (pending, key) {
            ('m', Key::Char(c)) if c.is_ascii_lowercase() => {
                let (line, column) = self.buffer.point(self.vim_at());
                self.vim.marks.insert(c, (line, column, self.buffer.revision()));
            }
            (jump @ ('\'' | '`'), Key::Char(c)) => {
                let to = if c == '\'' || c == '`' { self.vim.jump_back } else { self.vim.marks.get(&c).copied() };
                if let Some(mark) = to {
                    let (line, column) = self.vim_mark_now(mark);
                    let line = line.min(self.vim_last_line());
                    let motion = if jump == '\'' {
                        Motion { to: self.first_non_blank(line), linewise: true, inclusive: false }
                    } else {
                        Motion { to: self.buffer.offset(line, column), linewise: false, inclusive: false }
                    };
                    self.vim_note_jump();
                    self.vim_go(motion, operator, window, cx);
                }
            }
            ('"', Key::Char(c)) if c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '*' | '"') => {
                self.vim.register_name = (c != '"').then_some(c);
                self.vim.name_fresh = true;
                // The count before it goes on to the command.
                self.vim.count = count;
            }
            ('q', Key::Char(c)) if c.is_ascii_alphanumeric() => {
                self.vim.recording = Some((c.to_ascii_lowercase(), Vec::new()));
                cx.notify();
            }
            ('@', Key::Char(c)) => {
                let name = if c == '@' {
                    cx.try_global::<Shared>().and_then(|s| s.last_macro)
                } else {
                    Some(c.to_ascii_lowercase())
                };
                if let Some(name) = name {
                    self.vim_play(name, n, window, cx);
                }
            }
            ('g', Key::Char('g')) => {
                self.vim_note_jump();
                let last = self.vim_last_line();
                let to = count.map_or(0, |c| c.saturating_sub(1).min(last));
                let motion = Motion { to: self.first_non_blank(to), linewise: true, inclusive: false };
                self.vim_go(motion, operator, window, cx);
            }
            ('r', Key::Char(c)) if !self.vim.mode.visual() => {
                let head = self.selection.head;
                let line = self.buffer.point(head).0;
                if head + n <= self.line_end(line) {
                    self.edit(head..head + n, &c.to_string().repeat(n), EditKind::Other, cx);
                    self.vim_place(head + n - 1, cx);
                }
            }
            (kind @ ('f' | 'F' | 't' | 'T'), Key::Char(c)) => {
                self.vim.last_find = Some((kind, c));
                if let Some(motion) = self.vim_find(kind, c, n, false) {
                    self.vim_go(motion, operator, window, cx);
                }
            }
            (around @ ('i' | 'a'), Key::Char(c)) if operator.is_some() || self.vim.mode.visual() => {
                if let Some((range, linewise)) = self.vim_object(around == 'i', c) {
                    self.vim_on_object(range, linewise, operator, window, cx);
                }
            }
            _ => {}
        }
        true
    }

    /// Where a motion goes: an operator acts up to it; Visual mode's end moves; else the caret.
    fn vim_go(&mut self, motion: Motion, operator: Option<Operator>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(operator) = operator {
            self.vim_operate(operator, self.selection.head, motion, window, cx);
        } else if self.vim.mode.visual() {
            self.vim.head = self.vim_clamp(motion.to);
            self.vim_show_visual(cx);
        } else {
            self.vim_place(motion.to, cx);
        }
    }

    /// The `n`th `c` on the caret's line: f (on it), t (before it), F and T backwards.
    /// `again` (; and ,): t and T don't stop where they are.
    fn vim_find(&self, kind: char, c: char, n: usize, again: bool) -> Option<Motion> {
        let at = self.vim_at();
        let line = self.buffer.point(at).0;
        let (start, end) = (self.line_start(line), self.line_end(line));
        let rope = self.buffer.rope();
        let skip = usize::from(again && matches!(kind, 't' | 'T'));
        let mut found = None;
        let mut left = n;
        if matches!(kind, 'f' | 't') {
            for i in (at + 1 + skip).min(end)..end {
                if rope.char(i) == c {
                    left -= 1;
                    if left == 0 {
                        found = Some(i);
                        break;
                    }
                }
            }
        } else {
            for i in (start..at.saturating_sub(skip)).rev() {
                if rope.char(i) == c {
                    left -= 1;
                    if left == 0 {
                        found = Some(i);
                        break;
                    }
                }
            }
        }
        let i = found?;
        Some(match kind {
            'f' => Motion { to: i, linewise: false, inclusive: true },
            't' => Motion { to: i - 1, linewise: false, inclusive: true },
            'F' => Motion { to: i, linewise: false, inclusive: false },
            _ => Motion { to: i + 1, linewise: false, inclusive: false },
        })
    }

    /// The bracket matching the one at (or after) the caret on its line, for %.
    fn vim_matching_bracket(&self, at: usize) -> Option<usize> {
        let line = self.buffer.point(at).0;
        let rope = self.buffer.rope();
        let from = (at..self.line_end(line)).find(|&i| "()[]{}".contains(rope.char(i)))?;
        let c = rope.char(from);
        let (open, close, forward) = match c {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            _ => ('{', '}', false),
        };
        if forward { self.closing(from + 1, open, close) } else { self.opening(from, open, close) }
    }

    /// The `close` that ends what's open at `from` (scanning ahead, nested pairs skipped).
    fn closing(&self, from: usize, open: char, close: char) -> Option<usize> {
        let rope = self.buffer.rope();
        let mut depth = 0usize;
        for (i, c) in rope.chars_at(from.min(rope.len_chars())).enumerate().take(1_000_000) {
            if c == open {
                depth += 1;
            } else if c == close {
                if depth == 0 {
                    return Some(from + i);
                }
                depth -= 1;
            }
        }
        None
    }

    /// The `open` before `before` that isn't closed by then.
    fn opening(&self, before: usize, open: char, close: char) -> Option<usize> {
        let rope = self.buffer.rope();
        let mut depth = 0usize;
        let mut chars = rope.chars_at(before.min(rope.len_chars()));
        let mut i = before;
        while let Some(c) = chars.prev() {
            i -= 1;
            if before - i > 1_000_000 {
                return None;
            }
            if c == close {
                depth += 1;
            } else if c == open {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
        }
        None
    }

    /// A text object at the caret: `inner` (i) or around (a); `c` says which. The range,
    /// and whether it's whole lines.
    fn vim_object(&self, inner: bool, c: char) -> Option<(Range<usize>, bool)> {
        let at = self.vim_at();
        let rope = self.buffer.rope();
        let len = rope.len_chars();
        let (line, _) = self.buffer.point(at);
        let (start, end) = (self.line_start(line), self.line_end(line));
        match c {
            'w' | 'W' => {
                if start == end {
                    return None;
                }
                let at = at.min(end - 1);
                let big = c == 'W';
                let k = class(rope.char(at), big);
                let same = |i: usize| class(rope.char(i), big) == k;
                let mut a = at;
                while a > start && same(a - 1) {
                    a -= 1;
                }
                let mut b = at + 1;
                while b < end && same(b) {
                    b += 1;
                }
                if !inner {
                    let blank = |i: usize| matches!(rope.char(i), ' ' | '\t');
                    if k == 0 {
                        // On spaces: they and the word after.
                        if b < end {
                            let k2 = class(rope.char(b), big);
                            while b < end && class(rope.char(b), big) == k2 {
                                b += 1;
                            }
                        }
                    } else if b < end && blank(b) {
                        while b < end && blank(b) {
                            b += 1;
                        }
                    } else {
                        while a > start && blank(a - 1) {
                            a -= 1;
                        }
                    }
                }
                Some((a..b, false))
            }
            '"' | '\'' | '`' => {
                let quotes: Vec<usize> = (start..end)
                    .filter(|&i| rope.char(i) == c && (i == start || rope.char(i - 1) != '\\'))
                    .collect();
                let pair = quotes
                    .chunks(2)
                    .filter(|p| p.len() == 2)
                    .find(|p| p[0] <= at && at <= p[1])
                    .or_else(|| quotes.chunks(2).filter(|p| p.len() == 2).find(|p| p[0] > at))?;
                let (a, b) = (pair[0], pair[1]);
                Some(if inner { (a + 1..b, false) } else { (a..b + 1, false) })
            }
            '(' | ')' | 'b' | '[' | ']' | '{' | '}' | 'B' | '<' | '>' => {
                let (open, close) = match c {
                    '(' | ')' | 'b' => ('(', ')'),
                    '[' | ']' => ('[', ']'),
                    '{' | '}' | 'B' => ('{', '}'),
                    _ => ('<', '>'),
                };
                let here = rope.get_char(at.min(len.saturating_sub(1)))?;
                // On the opener: this pair; else (on the closer too) the one around.
                let a = if here == open { at } else { self.opening(at, open, close)? };
                let b = self.closing(a + 1, open, close)?;
                if !inner {
                    return Some((a..b + 1, false));
                }
                // A block over lines: its lines, not the breaks after { and before }.
                let mut from = a + 1;
                if rope.get_char(from) == Some('\r') {
                    from += 1;
                }
                if rope.get_char(from) == Some('\n') {
                    let to_line = self.buffer.point(b).0;
                    let closer_alone = self.first_non_blank(to_line) == b;
                    if closer_alone && to_line > self.buffer.point(a).0 + 1 {
                        return Some((from + 1..self.line_start(to_line), false));
                    }
                }
                Some((a + 1..b, false))
            }
            'p' => {
                let blank = |l: usize| self.buffer.line_text(l).trim().is_empty();
                let last = self.vim_last_line();
                let kind = blank(line);
                let mut first = line;
                while first > 0 && blank(first - 1) == kind {
                    first -= 1;
                }
                let mut end_line = line;
                while end_line < last && blank(end_line + 1) == kind {
                    end_line += 1;
                }
                if !inner {
                    while end_line < last && blank(end_line + 1) != kind {
                        end_line += 1;
                        if end_line < last && blank(end_line + 1) == kind {
                            break;
                        }
                    }
                }
                Some((self.line_start(first)..self.line_start(end_line + 1), true))
            }
            _ => None,
        }
    }

    /// An operator on a text object, or Visual mode taking it.
    fn vim_on_object(
        &mut self,
        range: Range<usize>,
        linewise: bool,
        operator: Option<Operator>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if linewise {
            let (first, last) = (self.buffer.point(range.start).0, self.buffer.point(range.end.saturating_sub(1)).0);
            match operator {
                Some(operator) => self.vim_operate_lines(operator, first, last, window, cx),
                None => {
                    self.vim.mode = Mode::VisualLine;
                    (self.vim.anchor, self.vim.head) = (self.line_start(first), self.line_start(last));
                    self.vim_show_visual(cx);
                }
            }
            return;
        }
        match operator {
            // Nothing inside (di( on "()"): c types there, d does nothing.
            Some(Operator::Change) if range.is_empty() => self.vim_insert_at(range.start, cx),
            Some(_) if range.is_empty() => {}
            Some(operator) => {
                // Exactly the object (a block's inner lines end with a line break).
                let motion = Motion { to: range.end, linewise: false, inclusive: false };
                self.vim_operate(operator, range.start, motion, window, cx);
            }
            None if !range.is_empty() => {
                if self.vim.mode == Mode::VisualLine {
                    self.vim.mode = Mode::Visual;
                }
                (self.vim.anchor, self.vim.head) = (range.start, range.end - 1);
                self.vim_show_visual(cx);
            }
            None => {}
        }
    }

    /// The steps added to the undo history since the change began, as one.
    fn vim_one_undo_step(&mut self) {
        if let Some(from) = self.vim.undo_from.take()
            && self.undo_stack.len() > from + 1
        {
            self.undo_stack.truncate(from + 1);
        }
    }

    /// `.`: the last change again (`count` in place of its own, when given).
    fn vim_repeat(&mut self, count: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let mut keys = self.vim.last_change.clone();
        if keys.is_empty() {
            return;
        }
        if let Some(count) = count {
            let digits = keys.iter().take_while(|k| matches!(k, Key::Char('0'..='9'))).count();
            keys.drain(..digits);
            let typed: Vec<Key> = count.to_string().chars().map(Key::Char).collect();
            keys.splice(0..0, typed);
        }
        self.vim.replaying = true;
        for key in keys {
            self.vim_step(key, window, cx);
        }
        if self.vim.mode == Mode::Insert {
            if let Some(text) = self.vim.last_insert.clone() {
                let at = self.selection.head;
                self.edit(at..at, &text, EditKind::Other, cx);
                // Typed as many times as it was (3ifoo, 2o): what's typed again from here.
                self.vim.insert_from = Some(at);
            }
            self.vim_leave_insert(cx);
        }
        self.vim.replaying = false;
        self.vim.register_name = None;
        self.vim_one_undo_step();
    }

    /// n and N: the next match of the search (backwards: `forward` false), the caret on it.
    fn vim_next_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let head = self.selection.head;
        // From just after the caret: the match it's on isn't the next.
        self.selection = Selection::caret(if forward { head + 1 } else { head });
        self.step_match(forward, cx);
        let at = if self.selection.is_empty() { head } else { self.selection.range().start };
        self.vim_place(at, cx);
    }

    /// * and #: the word at the caret, searched for as a whole word.
    fn vim_search_word(&mut self, back: bool, count: usize, cx: &mut Context<Self>) {
        let Some((range, _)) = self.vim_object(true, 'w').filter(|(r, _)| !r.is_empty()) else { return };
        let word = self.buffer.slice(range.clone());
        if word.trim().is_empty() {
            return;
        }
        crate::find_bar::remember_search(&word, cx);
        self.vim.search_back = back;
        self.selection = Selection { anchor: range.start, head: range.end };
        let query = SearchQuery { text: word, regex: false, whole_word: true, case_sensitive: true };
        self.set_search(query, cx);
        self.selection = Selection::caret(range.start);
        for _ in 0..count {
            self.vim_next_match(!back, cx);
        }
    }

    /// The find bar opened by / or ? closes: on the match found (Enter), or back where it
    /// started (Esc).
    pub fn vim_search_done(&mut self, found: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Typing ended by it: nothing of it to type again.
        if self.vim.mode == Mode::Insert {
            self.vim.insert_repeat = None;
            self.vim.insert_from = None;
            self.vim.insert_moved = false;
        }
        let before = self.vim.before_search.take();
        // ? : the match before where it started (find looks forward as you type).
        if found
            && let Some(before) = before
            && self.vim.search_back
            && self.search.is_some()
        {
            self.selection = Selection::caret(before);
            self.step_match(false, cx);
        }
        let at = match before {
            Some(before) if !found => before,
            _ => self.selection.range().start,
        };
        self.close_find(window, cx);
        self.vim.mode = Mode::Normal;
        self.vim_place(at, cx);
    }

    /// :N: line N, on its first character.
    /// Whether the find bar is open for / or ?.
    pub fn vim_searching(&self) -> bool {
        self.vim.before_search.is_some()
    }

    pub fn vim_go_to_line(&mut self, line: usize, cx: &mut Context<Self>) {
        let line = line.saturating_sub(1).min(self.vim_last_line());
        self.vim.mode = Mode::Normal;
        let at = self.first_non_blank(line);
        self.vim_place(at, cx);
    }

    /// :s/a/b/ (this line) and :%s/a/b/g (every line): `a` a regular expression; `b` with
    /// \1 and & for what it found. False when it isn't one.
    pub fn vim_substitute(&mut self, command: &str, cx: &mut Context<Self>) -> Result<usize, String> {
        let (every_line, rest) = match command.strip_prefix('%') {
            Some(rest) => (true, rest),
            None => (false, command),
        };
        let rest = rest.strip_prefix("s").ok_or_else(|| "not a substitution".to_string())?;
        let mut chars = rest.chars();
        let sep = chars.next().filter(|c| !c.is_alphanumeric() && *c != ' ').ok_or("not a substitution")?;
        let parts: Vec<String> = split_unescaped(chars.as_str(), sep);
        let (pattern, replacement, flags) = match parts.as_slice() {
            [p] => (p.clone(), String::new(), String::new()),
            [p, r] => (p.clone(), r.clone(), String::new()),
            [p, r, f, ..] => (p.clone(), r.clone(), f.clone()),
            _ => return Err("not a substitution".into()),
        };
        let pattern = vim_pattern(&pattern);
        let pattern = if flags.contains('i') { format!("(?i){pattern}") } else { pattern };
        let regex = Regex::new(&pattern).map_err(|_| format!("Can't search for {pattern}"))?;
        let replacement = vim_replacement(&replacement);
        let all = flags.contains('g');
        let lines = if every_line {
            0..=self.vim_last_line()
        } else {
            let line = self.buffer.point(self.selection.head).0;
            line..=line
        };
        let mut edits = Vec::new();
        let mut count = 0;
        for line in lines {
            let text = self.buffer.line_text(line);
            let found = if all { regex.find_iter(&text).count() } else { usize::from(regex.is_match(&text)) };
            if found == 0 {
                continue;
            }
            count += found;
            let new = if all { regex.replace_all(&text, replacement.as_str()) } else { regex.replace(&text, replacement.as_str()) };
            edits.push((self.line_start(line)..self.line_end(line), new.into_owned()));
        }
        if count == 0 {
            return Err(format!("Not found: {}", parts.first().cloned().unwrap_or_default()));
        }
        let first = edits[0].0.start;
        self.apply_char_edits(edits, cx);
        self.vim.mode = Mode::Normal;
        let line = self.buffer.point(first).0;
        let at = self.first_non_blank(line);
        self.vim_place(at, cx);
        Ok(count)
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
    fn vim_yank(&mut self, text: String, linewise: bool, yanked: bool, cx: &mut Context<Self>) {
        self.vim_keep(Register { text, linewise, block: false }, yanked, cx);
    }

    /// `register` kept (see `vim_yank`): in the register named, the unnamed one and the
    /// clipboard; in "0 too when it was yanked.
    fn vim_keep(&mut self, register: Register, yanked: bool, cx: &mut Context<Self>) {
        let name = self.vim.register_name;
        // "_: kept nowhere.
        if name == Some('_') {
            return;
        }
        let linewise = register.linewise;
        match name {
            // "A: added to "a.
            Some(c) if c.is_ascii_uppercase() => {
                let kept = cx
                    .default_global::<Shared>()
                    .registers
                    .entry(c.to_ascii_lowercase())
                    .or_insert(Register { linewise, ..Register::default() });
                kept.text.push_str(&register.text);
                kept.linewise |= linewise;
            }
            Some(c) if c.is_ascii_alphanumeric() => {
                cx.default_global::<Shared>().registers.insert(c, register.clone());
            }
            _ => {}
        }
        if yanked {
            cx.default_global::<Shared>().registers.insert('0', register.clone());
        }
        #[cfg(not(test))]
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(register.text.clone()));
        #[cfg(test)]
        let _ = cx;
        cx.default_global::<Shared>().unnamed = Some(register);
    }

    /// What p puts: the clipboard when something else was copied since, else the register.
    fn vim_register(&self, cx: &App) -> Option<Register> {
        match self.vim.register_name {
            Some('_') => return None,
            Some(c) if c.is_ascii_alphanumeric() => {
                return cx.try_global::<Shared>().and_then(|s| s.registers.get(&c.to_ascii_lowercase())).cloned();
            }
            _ => {}
        }
        #[cfg(not(test))]
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
            && cx.try_global::<Shared>().and_then(|s| s.unnamed.as_ref()).is_none_or(|r| r.text != text)
        {
            let linewise = text.ends_with('\n');
            return Some(Register { text, linewise, block: false });
        }
        #[cfg(test)]
        let _ = cx;
        cx.try_global::<Shared>().and_then(|s| s.unnamed.clone())
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
        // Inclusive of the character there, not of a line break (d$ on an empty line).
        let on_text = !matches!(self.buffer.char_at(b), Some('\n' | '\r') | None);
        let end = if motion.inclusive && on_text { b + 1 } else { b };
        let range = a..end;
        match operator {
            Operator::Delete | Operator::Change => {
                let text = self.buffer.slice(range.clone());
                if !text.is_empty() {
                    self.vim_yank(text, false, false, cx);
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
                self.vim_yank(text, false, true, cx);
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
                self.vim_yank(text, true, true, cx);
                let at = if self.buffer.point(self.selection.head).0 > first { start } else { self.selection.head };
                self.vim_place(at.max(start), cx);
            }
            Operator::Delete => {
                self.vim_yank(text, true, false, cx);
                // The last lines: the break before them goes with them.
                let range = if after >= self.buffer.len_chars() && after == self.line_end(last) && first > 0 {
                    self.line_end(first - 1)..after
                } else {
                    start..after
                };
                if !range.is_empty() {
                    self.edit(range, "", EditKind::Other, cx);
                }
                let line = first.min(self.vim_last_line());
                let at = self.first_non_blank(line);
                self.vim_place(at, cx);
            }
            Operator::Change => {
                self.vim_yank(text, true, false, cx);
                // Its indentation stays, the text goes.
                let indent = self.first_non_blank(first) - start;
                let range = start + indent..self.line_end(last);
                if !range.is_empty() {
                    self.edit(range, "", EditKind::Other, cx);
                }
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
        if register.block {
            return self.vim_put_block(&register.text, after, count, cx);
        }
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

    /// A block put back as one: each of its lines on a line from the caret's down, at the
    /// caret's column (after it, for p), short lines filled with spaces to reach it, lines
    /// added past the end.
    fn vim_put_block(&mut self, text: &str, after: bool, count: usize, cx: &mut Context<Self>) {
        let (line, column) = self.buffer.point(self.selection.head);
        let shown = self.display_column(line, column) + usize::from(after && self.buffer.line_len(line) > 0);
        let last = self.vim_last_line();
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut added = String::new();
        for (i, part) in text.split('\n').enumerate() {
            let part = part.repeat(count.max(1));
            let target = line + i;
            if target <= last {
                let (at, short) = self.column_char(target, shown);
                let at = self.line_start(target) + at;
                edits.push((at..at, format!("{}{part}", " ".repeat(short))));
            } else {
                added.push_str(&format!("{}{}{part}", self.style.line_ending.text(), " ".repeat(shown)));
            }
        }
        if !added.is_empty() {
            let end = self.line_end(last);
            // (Right after what goes at the end of the last line, if anything does.)
            match edits.iter_mut().find(|(r, _)| r.start == end) {
                Some((_, text)) => text.push_str(&added),
                None => edits.push((end..end, added)),
            }
        }
        self.apply_char_edits(edits, cx);
        let at = self.line_start(line) + self.column_char(line, shown).0;
        self.vim_place(at, cx);
    }

    fn vim_start_visual(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.vim.block_to_end = false;
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
        if self.vim.mode == Mode::VisualBlock {
            // A selection on each line of the block, between its columns: Null's cursors.
            let cursors: Vec<(Cursor, bool)> =
                self.vim_block_ranges().into_iter().map(|(line, range)| (Cursor::new(Selection { anchor: range.start, head: range.end }), line == self.buffer.point(head).0)).collect();
            self.set_cursors(cursors);
            self.vim.shown = Some((self.selection, self.buffer.revision()));
            self.goal_column = None;
            return self.touch(cx);
        }
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
        self.vim.shown = Some((self.selection, self.buffer.revision()));
        self.goal_column = None;
        self.touch(cx);
    }

    fn vim_end_visual(&mut self, cx: &mut Context<Self>) {
        self.single_cursor();
        self.vim.mode = Mode::Normal;
        let at = self.vim.head;
        self.vim_place(at, cx);
    }

    /// A key in Visual mode: a motion moves the head; an operator acts on the selection.
    fn vim_visual(&mut self, key: Key, count: Option<usize>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Key::Char(c @ ('g' | 'i' | 'a' | 'f' | 'F' | 't' | 'T' | '\'' | '`' | '"')) = key {
            self.vim.count = count;
            self.vim.pending = Some(c);
            return true;
        }
        if let Some(motion) = self.vim_motion(&key, count, false) {
            // $ in a block: to every line's end; a move across a line ends that.
            if !matches!(key, Key::Char('j' | 'k' | 'G' | '+' | '-') | Key::Ctrl(_)) {
                self.vim.block_to_end = key == Key::Char('$');
            }
            let goal = self.vim.goal;
            self.vim.head = self.vim_clamp(motion.to);
            self.vim.goal = goal;
            self.vim_show_visual(cx);
            return true;
        }
        if self.vim.mode == Mode::VisualBlock {
            return self.vim_block(key, window, cx);
        }
        if key == Key::Ctrl('v') {
            self.vim.mode = Mode::VisualBlock;
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
                self.selection = Selection { anchor: self.line_start(first), head: self.line_end(last.min(self.vim_last_line())) };
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
                    // (Into the unnamed one only: "ap keeps "a as it was.)
                    let name = self.vim.register_name.take();
                    self.vim_yank(replaced, lines, false, cx);
                    self.vim.register_name = name;
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

/// `text` cut at each `sep` not after a backslash (a `\sep` is kept as `sep`).
fn split_unescaped(text: &str, sep: char) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(n) if n == sep => parts.last_mut().unwrap().push(n),
                Some(n) => {
                    parts.last_mut().unwrap().push('\\');
                    parts.last_mut().unwrap().push(n);
                }
                None => parts.last_mut().unwrap().push('\\'),
            }
        } else if c == sep {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    if parts.last().is_some_and(String::is_empty) && parts.len() > 1 {
        parts.pop();
    }
    parts
}

/// Vim's pattern as the regex crate reads it: \( \) \| \+ \? \{ \} are its groups and
/// counts (bare, they're themselves), \< \> a word's edges.
fn vim_pattern(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(n @ ('(' | ')' | '|' | '+' | '?' | '{' | '}')) => out.push(n),
                Some('<' | '>') => out.push_str(r"\b"),
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push_str(r"\\"),
            },
            '(' | ')' | '|' | '+' | '?' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Vim's replacement (\1, &, \&) as the regex crate writes it (${1}, ${0}, &).
fn vim_replacement(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(d @ '0'..='9') => out.push_str(&format!("${{{d}}}")),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            },
            '&' => out.push_str("${0}"),
            '$' => out.push_str("$$"),
            _ => out.push(c),
        }
    }
    out
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
        e.read_with(cx, |e, cx| {
            let mut text = e.buffer.to_string();
            text.insert(text.char_indices().nth(e.shown_caret(cx)).map_or(text.len(), |(i, _)| i), '|');
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
        assert_eq!(cx.update(|_, cx| cx.global::<Shared>().unnamed.clone().unwrap().text), "ONE two");
        cx.simulate_keystrokes("v escape");
        assert!(e.read_with(cx, |e, _| e.selection.is_empty()));
    }

    #[gpui::test]
    fn text_objects(cx: &mut TestAppContext) {
        let (e, cx) = vim("call(one, \"two |words\", [x])\n", cx);
        cx.simulate_keystrokes("c i w W escape");
        assert_eq!(shown(&e, cx), "call(one, \"two |W\", [x])\n");
        cx.simulate_keystrokes("d i \"");
        assert_eq!(shown(&e, cx), "call(one, \"|\", [x])\n");
        cx.simulate_keystrokes("d a (");
        assert_eq!(shown(&e, cx), "cal|l\n");
        let (e, cx) = vim("fn f() {\n    let a = 1;\n    |let b = 2;\n}\n\nnext\n", &mut cx.cx);
        cx.simulate_keystrokes("d i {");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "fn f() {\n}\n\nnext\n", "a block's lines");
        cx.simulate_keystrokes("u g g y a p G p");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()).matches("fn f()").count(), 2, "a paragraph, and the blank after");
        // In Visual mode, i w selects the word.
        let (e, cx) = vim("say he|llo there\n", &mut cx.cx);
        cx.simulate_keystrokes("v i w");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.slice(e.selection.range())), "hello");
    }

    #[gpui::test]
    fn find_on_the_line_and_brackets(cx: &mut TestAppContext) {
        let (e, cx) = vim("|a(b, c(d), e)\n", cx);
        cx.simulate_keystrokes("f ,");
        assert_eq!(shown(&e, cx), "a(b|, c(d), e)\n");
        cx.simulate_keystrokes(";");
        assert_eq!(shown(&e, cx), "a(b, c(d)|, e)\n");
        cx.simulate_keystrokes(",");
        assert_eq!(shown(&e, cx), "a(b|, c(d), e)\n");
        cx.simulate_keystrokes("0 %");
        assert_eq!(shown(&e, cx), "a(b, c(d), e|)\n", "% from before the bracket");
        cx.simulate_keystrokes("%");
        assert_eq!(shown(&e, cx), "a|(b, c(d), e)\n");
        cx.simulate_keystrokes("d t )");
        assert_eq!(shown(&e, cx), "a|), e)\n", "up to the first )");
    }

    #[gpui::test]
    fn dot_repeats_the_last_change(cx: &mut TestAppContext) {
        let (e, cx) = vim("|one two three four\n", cx);
        cx.simulate_keystrokes("d w .");
        assert_eq!(shown(&e, cx), "|three four\n");
        cx.simulate_keystrokes("c w T H R E E escape w .");
        assert_eq!(shown(&e, cx), "THREE THRE|E\n", "with what was typed");
        cx.simulate_keystrokes("u");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "THREE four\n");
        cx.simulate_keystrokes("0 A ! escape j .");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "THREE four!!\n");
    }

    #[gpui::test]
    fn searching(cx: &mut TestAppContext) {
        let (e, cx) = vim("|item = items[0]\nfor item in items:\n    print(item)\n", cx);
        cx.simulate_keystrokes("*");
        assert_eq!(shown(&e, cx), "item = items[0]\nfor |item in items:\n    print(item)\n", "the whole word");
        cx.simulate_keystrokes("n");
        assert_eq!(shown(&e, cx), "item = items[0]\nfor item in items:\n    print(|item)\n");
        cx.simulate_keystrokes("N N");
        assert_eq!(shown(&e, cx), "|item = items[0]\nfor item in items:\n    print(item)\n");
        // :s and :%s.
        e.update(cx, |e, cx| {
            assert_eq!(e.vim_substitute("%s/item\\(s\\?\\)/thing\\1/g", cx), Ok(5));
            assert_eq!(e.buffer.to_string(), "thing = things[0]\nfor thing in things:\n    print(thing)\n");
            assert_eq!(e.vim_substitute("s/thing/&&/", cx), Ok(1));
            assert!(e.buffer.to_string().starts_with("thingthing = things"));
            assert!(e.vim_substitute("%s/nothing here//", cx).is_err());
        });
    }

    #[gpui::test]
    fn slash_searches_with_the_find_bar(cx: &mut TestAppContext) {
        let (e, cx) = vim("|alpha\nbeta\ngamma beta\n", cx);
        cx.simulate_keystrokes("/");
        cx.simulate_input("beta");
        cx.simulate_keystrokes("enter");
        assert_eq!(shown(&e, cx), "alpha\n|beta\ngamma beta\n");
        e.read_with(cx, |e, cx| assert_eq!(e.vim_mode(cx), Some(Mode::Normal)));
        cx.simulate_keystrokes("n");
        assert_eq!(shown(&e, cx), "alpha\nbeta\ngamma |beta\n");
        // Esc: back where it started.
        cx.simulate_keystrokes("g g / g a m escape");
        assert_eq!(shown(&e, cx), "|alpha\nbeta\ngamma beta\n");
    }

    #[test]
    fn substitutions_read_as_vim_writes_them() {
        assert_eq!(split_unescaped("a\\/b/c/g", '/'), ["a/b", "c", "g"]);
        assert_eq!(vim_replacement("<\\1>&$"), "<${1}>${0}$$");
    }

    /// What the twenty-first review found.
    #[gpui::test]
    fn edges_found_in_review(cx: &mut TestAppContext) {
        // Visual's selection cut away (⌘X): its old range isn't used again, nor read past the end.
        let (e, cx) = vim("|hello world\n", cx);
        cx.simulate_keystrokes("v $");
        e.update(cx, |e, cx| e.edit(0..11, "", EditKind::Other, cx));
        cx.simulate_keystrokes("b x");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "\n");
        // A bracket object in an empty file.
        e.update(cx, |e, cx| e.edit(0..1, "", EditKind::Other, cx));
        cx.simulate_keystrokes("d i (");
        // cw on a word's last character changes only it.
        let (e, cx) = vim("fo|o(bar\nbaz(z\n", &mut cx.cx);
        cx.simulate_keystrokes("c w X escape");
        assert_eq!(shown(&e, cx), "fo|X(bar\nbaz(z\n");
        // ct. is a change of its own for `.`.
        cx.simulate_keystrokes("0 c t ( Y escape j 0 .");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "Y(bar\nY(z\n", "ct( again, on the next line");
        // 3J joins three lines; d$ on an empty line leaves the break.
        let (e, cx) = vim("|a\nb\nc\n\nd\n", &mut cx.cx);
        cx.simulate_keystrokes("3 J");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "a b c\n\nd\n");
        cx.simulate_keystrokes("j d $");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "a b c\n\nd\n");
        // A click after a line's end: a appends there, not on the next line.
        e.update(cx, |e, _| e.selection = Selection::caret(5));
        cx.simulate_keystrokes("a X escape");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "a b cX\n\nd\n");
    }

    /// Enter and Tab as macOS sends them (with their characters); Esc and ⌃ keys.
    #[gpui::test]
    fn keys_as_the_mac_sends_them(cx: &mut TestAppContext) {
        let (e, cx) = vim("|one\n  two\n", cx);
        let key = |key: &str, ch: Option<&str>| gpui::Keystroke { key: key.into(), key_char: ch.map(Into::into), ..Default::default() };
        let taken = e.update_in(cx, |e, window, cx| e.vim_key(&key("enter", Some("\n")), window, cx));
        assert!(taken);
        assert_eq!(shown(&e, cx), "one\n  |two\n", "Enter: the next line's first character");
        assert!(!e.update_in(cx, |e, window, cx| e.vim_key(&key("tab", Some("\t")), window, cx)), "Tab is Null's");
        assert!(!e.update_in(cx, |e, window, cx| e.vim_key(&key("escape", None), window, cx)), "Esc with nothing to cancel");
        // ⌃O (Null's open line) edits nothing here.
        cx.simulate_keystrokes("ctrl-o");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "one\n  two\n");
    }

    #[gpui::test]
    fn counts_on_typing_and_percent(cx: &mut TestAppContext) {
        let (e, cx) = vim("|x\n", cx);
        cx.simulate_keystrokes("3 i a b escape");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "abababx\n");
        cx.simulate_keystrokes("2 o - escape");
        assert_eq!(shown(&e, cx), "abababx\n-\n|-\n");
        let ten: String = (1..=10).map(|n| format!("line {n}\n")).collect();
        let (e, cx) = vim(&format!("|{ten}"), &mut cx.cx);
        cx.simulate_keystrokes("5 0 %");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.point(e.selection.head).0), 4, "half way: line 5");
    }

    #[gpui::test]
    fn marks_and_jumps_back(cx: &mut TestAppContext) {
        let (e, cx) = vim("one\n  t|wo\nthree\nfour\n", cx);
        cx.simulate_keystrokes("m a G");
        assert_eq!(shown(&e, cx), "one\n  two\nthree\n|four\n");
        cx.simulate_keystrokes("` a");
        assert_eq!(shown(&e, cx), "one\n  t|wo\nthree\nfour\n", "exactly where it was");
        cx.simulate_keystrokes("' '");
        assert_eq!(shown(&e, cx), "one\n  two\nthree\n|four\n", "back where the jump left");
        cx.simulate_keystrokes("' a");
        assert_eq!(shown(&e, cx), "one\n  |two\nthree\nfour\n", "the line's first character");
        cx.simulate_keystrokes("d ' a");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "one\nthree\nfour\n");
    }

    #[gpui::test]
    fn named_registers(cx: &mut TestAppContext) {
        let (e, cx) = vim("|alpha\nbeta\ngamma\n", cx);
        cx.simulate_keystrokes("\" a y y j \" b y y");
        cx.simulate_keystrokes("G \" a p");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "alpha\nbeta\ngamma\nalpha\n");
        // "_ keeps nothing: p still puts what was yanked.
        cx.simulate_keystrokes("\" _ d d g g p");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "alpha\nbeta\nbeta\ngamma\n");
        // "0: the last yank, whatever was deleted since.
        cx.simulate_keystrokes("G d d g g \" 0 P");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "beta\nalpha\nbeta\nbeta\n");
    }

    #[gpui::test]
    fn macros(cx: &mut TestAppContext) {
        let (e, cx) = vim("|a\nb\nc\nd\n", cx);
        cx.simulate_keystrokes("q a");
        e.read_with(cx, |e, _| assert_eq!(e.vim_recording(), Some('a')));
        cx.simulate_keystrokes("A ; escape j q");
        e.read_with(cx, |e, _| assert_eq!(e.vim_recording(), None));
        cx.simulate_keystrokes("2 @ a");
        cx.run_until_parked();
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "a;\nb;\nc;\nd\n");
        cx.simulate_keystrokes("@ @");
        cx.run_until_parked();
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "a;\nb;\nc;\nd;\n");
    }

    #[gpui::test]
    fn visual_block(cx: &mut TestAppContext) {
        let (e, cx) = vim("|abcd\nefgh\nij\n", cx);
        cx.simulate_keystrokes("l ctrl-v l j");
        assert_eq!(e.read_with(cx, |e, _| e.extra.len()), 1, "a selection on each line");
        e.read_with(cx, |e, cx| assert_eq!(e.vim_mode(cx), Some(Mode::VisualBlock)));
        cx.simulate_keystrokes("d");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "ad\neh\nij\n");
        cx.simulate_keystrokes("g g ctrl-v j I # escape");
        assert_eq!(e.read_with(cx, |e, _| e.buffer.to_string()), "#ad\n#eh\nij\n", "typed on each line");
        e.read_with(cx, |e, cx| {
            assert_eq!(e.vim_mode(cx), Some(Mode::Normal));
            assert!(e.extra.is_empty());
        });
    }

    /// What the twenty-second review found.
    #[gpui::test]
    fn registers_marks_and_macros_as_vim_has_them(cx: &mut TestAppContext) {
        let (e, cx) = vim("|one\ntwo\nthree\nfour\n", cx);
        let text = |e: &Entity<Editor>, cx: &mut gpui::VisualTestContext| e.read_with(cx, |e, _| e.buffer.to_string());
        // A count before the name: the name is for that command only.
        cx.simulate_keystrokes("2 \" a y y x G \" a p");
        assert_eq!(text(&e, cx), "ne\ntwo\nthree\nfour\none\ntwo\n");
        // Visual "ap: "a keeps what it had.
        cx.simulate_keystrokes("g g v e \" a p g g j v e \" a p");
        assert_eq!(text(&e, cx).lines().take(2).collect::<Vec<_>>(), ["one", "one"], "{}", text(&e, cx));
        // A mark follows the lines above it going.
        let (e, cx) = vim("a\nb\nc\n|mark\n", &mut cx.cx);
        cx.simulate_keystrokes("m a g g d d G ' a");
        assert_eq!(shown(&e, cx), "b\nc\n|mark\n");
        // A macro played from another plays there, not after it; its keys aren't recorded twice.
        let (e, cx) = vim("|x\ny\n", &mut cx.cx);
        cx.simulate_keystrokes("q b A ! escape q");
        cx.simulate_keystrokes("q a j @ b");
        cx.run_until_parked();
        cx.simulate_keystrokes("A ? escape q");
        assert_eq!(text(&e, cx), "x!\ny!?\n");
        cx.simulate_keystrokes("g g @ a");
        cx.run_until_parked();
        assert_eq!(text(&e, cx), "x!\ny!?!?\n", "j, then b's !, then ?");
        // Registers are every file's.
        let (other, cx) = vim("|z\n", &mut cx.cx);
        cx.simulate_keystrokes("@ b");
        cx.run_until_parked();
        assert_eq!(text(&other, cx), "z!\n");
    }

    #[gpui::test]
    fn counted_typing_as_vim_has_it(cx: &mut TestAppContext) {
        let (e, cx) = vim("|ab\ncd\n", cx);
        let text = |e: &Entity<Editor>, cx: &mut gpui::VisualTestContext| e.read_with(cx, |e, _| e.buffer.to_string());
        // Moved while typing: not typed again (what's between isn't all typed).
        cx.simulate_keystrokes("3 i x right escape");
        assert_eq!(text(&e, cx), "xab\ncd\n");
        // . types it as many times as it was.
        cx.simulate_keystrokes("j 0 3 i - escape g g 0 .");
        assert_eq!(text(&e, cx), "---xab\n---cd\n");
        // Block I: lines too short to reach the block are left alone.
        let (e, cx) = vim("ab|cd\nx\nabcd\n", &mut cx.cx);
        cx.simulate_keystrokes("ctrl-v j j I # escape");
        assert_eq!(text(&e, cx), "ab#cd\nx\nab#cd\n");
    }

    /// A block copied goes back as a block; $ takes every line to its end; tabs count as
    /// the room they take.
    #[gpui::test]
    fn blocks_go_back_as_blocks(cx: &mut TestAppContext) {
        let (e, cx) = vim("|ab12\ncd34\nef\n", cx);
        let text = |e: &Entity<Editor>, cx: &mut gpui::VisualTestContext| e.read_with(cx, |e, _| e.buffer.to_string());
        cx.simulate_keystrokes("ctrl-v j l y");
        cx.simulate_keystrokes("j j $ p");
        assert_eq!(text(&e, cx), "ab12\ncd34\nefab\n  cd\n", "after the column, a line added below");
        cx.simulate_keystrokes("g g ctrl-v j $ d");
        assert_eq!(text(&e, cx), "\n\nefab\n  cd\n", "$: every line to its end");
        // Columns as shown: a tab takes the room it's drawn in.
        let (e, cx) = vim("|\tab\nxy\n", &mut cx.cx);
        e.read_with(cx, |e, _| {
            let tab = e.style.indent.width().max(1);
            assert_eq!(e.display_column(0, 1), tab, "after the tab");
            assert_eq!(e.column_char(0, tab + 1), (2, 0), "b, past the tab and a");
            assert_eq!(e.column_char(0, tab + 5), (3, 3), "past the line's end: 3 spaces short");
            assert_eq!(e.column_char(1, 1), (1, 0));
        });
    }

    #[test]
    fn patterns_read_as_vim_writes_them() {
        assert_eq!(vim_pattern(r"item\(s\?\)"), r"item(s?)");
        assert_eq!(vim_pattern(r"f(x)+"), r"f\(x\)\+");
        assert_eq!(vim_pattern(r"\<word\>"), r"\bword\b");
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
