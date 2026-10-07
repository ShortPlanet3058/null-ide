use crate::fonts::CodeFont;
use crate::search::SearchQuery;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use crate::ui;
use futures::StreamExt;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle, KeyBinding,
    ScrollStrategy, SharedString, StyledText, Subscription, Task, UniformListScrollHandle, Window, actions, div,
    prelude::*, px, uniform_list,
};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

actions!(project_search, [NextResult, PreviousResult, OpenResult, ReplaceAllResults]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("ProjectSearch");
    cx.bind_keys([
        KeyBinding::new("down", NextResult, ctx),
        KeyBinding::new("ctrl-n", NextResult, ctx),
        KeyBinding::new("up", PreviousResult, ctx),
        KeyBinding::new("ctrl-p", PreviousResult, ctx),
        KeyBinding::new("enter", OpenResult, ctx),
        KeyBinding::new("secondary-enter", ReplaceAllResults, ctx),
    ]);
}

/// Files with unsaved changes in a tab, by path: searching reads these instead of the
/// disk, so results match what's on screen. Kept up to date by the workspace.
#[derive(Default)]
pub struct UnsavedFiles(pub std::collections::HashMap<PathBuf, ropey::Rope>);

impl gpui::Global for UnsavedFiles {}

/// Results arrive in batches this often while a search runs.
const BATCH_EVERY: Duration = Duration::from_millis(80);

/// What a running search reports.
enum Found {
    Files(Vec<FileResult>),
    Done { truncated: bool },
}

/// Searching stops after this many matches.
const MAX_MATCHES: usize = 2_000;
/// Bigger files are skipped: they're almost never source code.
const MAX_FILE_SIZE: u64 = 1024 * 1024;
/// Wait this long after the last keystroke before searching.
const DEBOUNCE: Duration = Duration::from_millis(150);
const ROW_HEIGHT: f32 = ui::ROW_SM;
/// Long lines are cut down to this many characters around the match.
const PREVIEW_CHARS: usize = 160;

pub struct LineMatch {
    /// Zero-based line number.
    pub line: usize,
    /// Char columns of the first match on the line.
    pub columns: Range<usize>,
    preview: SharedString,
    /// Byte ranges in `preview` to highlight.
    highlights: Vec<Range<usize>>,
}

/// What Find TODOs looks for: the notes left in code, as whole words, in capitals.
pub fn todo_query() -> SearchQuery {
    SearchQuery { text: r"\b(TODO|FIXME|HACK|XXX)\b".into(), case_sensitive: true, whole_word: false, regex: true }
}

pub struct FileResult {
    path: PathBuf,
    relative: String,
    matches: Vec<LineMatch>,
}

enum Row {
    File(usize),
    Match(usize, usize),
}

enum Status {
    Idle,
    Searching,
    Done { files: usize, matches: usize, truncated: bool },
    Invalid,
}

pub enum ProjectSearchEvent {
    /// `keep_focus`: opened from the keyboard, so the search keeps the focus to go on
    /// to the next result.
    Open { path: PathBuf, line: usize, columns: Range<usize>, query: SearchQuery, keep_focus: bool },
    /// Replace the matches in these files (only on the given line, when there is one).
    Replace { query: SearchQuery, replacement: String, targets: Vec<(PathBuf, Option<usize>)> },
}

/// Search across every file in the project. Lives in the sidebar.
pub struct ProjectSearch {
    root: PathBuf,
    input: Entity<TextInput>,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    results: Vec<FileResult>,
    rows: Vec<Row>,
    status: Status,
    query: SearchQuery,
    task: Option<Task<()>>,
    /// Set to stop the search running in the background when a new one starts.
    cancel: Arc<AtomicBool>,
    /// The result row chosen from the keyboard.
    selected: Option<usize>,
    scroll: UniformListScrollHandle,
    replace_input: Entity<TextInput>,
    /// Which files to search (`*.rs, src/, !tests`), under the replace field.
    files_input: Entity<TextInput>,
    /// The replace field and the files field show (opened with the chevron, or ⌘⇧H).
    show_replace: bool,
    /// The query compiled, for previewing replacements.
    compiled: Option<regex::Regex>,
    _subscription: Subscription,
    _replace_subscription: Subscription,
    _files_subscription: Subscription,
}

impl EventEmitter<ProjectSearchEvent> for ProjectSearch {}

impl ProjectSearch {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Search in project", cx));
        let subscription = cx.subscribe(&input, |this, _, TextInputEvent::Changed, cx| this.search(cx));
        let replace_input = cx.new(|cx| TextInput::new("Replace with", cx));
        let replace_subscription = cx.subscribe(&replace_input, |_, _, TextInputEvent::Changed, cx| cx.notify());
        let files_input = cx.new(|cx| TextInput::new("In files: *.rs, src/, !tests", cx));
        let files_subscription = cx.subscribe(&files_input, |this, _, TextInputEvent::Changed, cx| this.search(cx));
        Self {
            root,
            input,
            case_sensitive: false,
            whole_word: false,
            regex: false,
            results: Vec::new(),
            rows: Vec::new(),
            status: Status::Idle,
            query: SearchQuery::default(),
            task: None,
            cancel: Arc::default(),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            replace_input,
            files_input,
            show_replace: false,
            compiled: None,
            _subscription: subscription,
            _replace_subscription: replace_subscription,
            _files_subscription: files_subscription,
        }
    }

    /// Opens the replace field (⌘⇧H), focused when the search already has text.
    pub fn show_replace(&mut self, text: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.show_replace = true;
        let searching = text.is_some() || !self.input.read(cx).text().is_empty();
        self.focus(text, window, cx);
        if searching {
            window.focus(&self.replace_input.focus_handle(cx));
        }
        cx.notify();
    }

    /// Runs the search again (after a replace changed the files).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.search(cx);
    }

    fn replacement(&self, cx: &App) -> String {
        self.replace_input.read(cx).text().to_string()
    }

    /// ⌘↵ or the button: every match in every file found.
    fn replace_all(&mut self, _: &ReplaceAllResults, _: &mut Window, cx: &mut Context<Self>) {
        if !self.show_replace || self.results.is_empty() {
            return;
        }
        let targets = self.results.iter().map(|f| (f.path.clone(), None)).collect();
        cx.emit(ProjectSearchEvent::Replace { query: self.query.clone(), replacement: self.replacement(cx), targets });
    }

    fn replace_line(&mut self, file: usize, line_match: usize, cx: &mut Context<Self>) {
        let file = &self.results[file];
        let line = file.matches[line_match].line;
        cx.emit(ProjectSearchEvent::Replace {
            query: self.query.clone(),
            replacement: self.replacement(cx),
            targets: vec![(file.path.clone(), Some(line))],
        });
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.root = root;
        self.search(cx);
    }

    /// Whether something is typed in the field.
    pub fn has_query(&self, cx: &App) -> bool {
        !self.input.read(cx).text().is_empty()
    }

    /// Focuses the field, optionally replacing its text.
    pub fn focus(&mut self, text: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| match text {
            Some(text) => input.set_text(&text, cx),
            None => input.select_all_text(cx),
        });
        window.focus(&self.input.focus_handle(cx));
    }

    /// Searches for `query` (its choices too: case, word, pattern), the field showing it.
    pub fn search_for(&mut self, query: SearchQuery, window: &mut Window, cx: &mut Context<Self>) {
        self.case_sensitive = query.case_sensitive;
        self.whole_word = query.whole_word;
        self.regex = query.regex;
        self.input.update(cx, |input, cx| input.set_text(&query.text, cx));
        window.focus(&self.input.focus_handle(cx));
        self.search(cx);
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let query = SearchQuery {
            text: self.input.read(cx).text().to_string(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
        };
        self.query = query.clone();
        self.compiled = query.build().ok();
        self.selected = None;
        self.cancel.store(true, Ordering::Relaxed);
        if query.text.is_empty() {
            self.task = None;
            self.set_results(Vec::new(), Status::Idle, cx);
            return;
        }
        if query.build().is_err() {
            self.task = None;
            self.set_results(Vec::new(), Status::Invalid, cx);
            return;
        }
        self.status = Status::Searching;
        cx.notify();
        let root = self.root.clone();
        // The files field counts only while it shows.
        let files = if self.show_replace { self.files_input.read(cx).text().to_string() } else { String::new() };
        let unsaved = cx.try_global::<UnsavedFiles>().map(|u| u.0.clone()).unwrap_or_default();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        // Results show as they're found, file by file in order; the previous results stay
        // until the first new ones arrive, so the list doesn't flash empty while typing.
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let (tx, mut rx) = futures::channel::mpsc::unbounded();
            cx.background_executor()
                .spawn(async move {
                    search_files(&root, &query, &files, &unsaved, &cancel, |found| {
                        tx.unbounded_send(found).ok();
                    })
                })
                .detach();
            let mut fresh = true;
            while let Some(found) = rx.next().await {
                let ok = this.update(cx, |this, cx| {
                    let mut results = if fresh { Vec::new() } else { std::mem::take(&mut this.results) };
                    fresh = false;
                    let status = match found {
                        Found::Files(files) => {
                            results.extend(files);
                            Status::Searching
                        }
                        Found::Done { truncated } => {
                            let matches = results.iter().map(|f| f.matches.len()).sum();
                            Status::Done { files: results.len(), matches, truncated }
                        }
                    };
                    this.set_results(results, status, cx);
                });
                if ok.is_err() {
                    break;
                }
            }
        }));
    }

    /// Moves the keyboard choice to the next (or previous) match, skipping file names.
    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let is_match = |ix: &usize| matches!(self.rows[*ix], Row::Match(..));
        let next = match (self.selected, forward) {
            (None, true) => (0..self.rows.len()).find(is_match),
            (None, false) => (0..self.rows.len()).rev().find(is_match),
            (Some(at), true) => (at + 1..self.rows.len()).find(is_match).or(Some(at)),
            (Some(at), false) => (0..at).rev().find(is_match).or(Some(at)),
        };
        if let Some(ix) = next {
            self.selected = Some(ix);
            // Scrolls only as far as needed; going up, a file's first match brings its name along.
            let shown = if !forward && ix > 0 && matches!(self.rows[ix - 1], Row::File(_)) { ix - 1 } else { ix };
            self.scroll.scroll_to_item(shown, ScrollStrategy::Top);
            cx.notify();
        }
    }

    fn next_result(&mut self, _: &NextResult, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn previous_result(&mut self, _: &PreviousResult, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    /// ↵ opens the chosen match (the first, if none is chosen) and stays here for the next.
    fn open_result(&mut self, _: &OpenResult, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_none() {
            self.step(true, cx);
        }
        if let Some(Row::Match(f, m)) = self.selected.and_then(|ix| self.rows.get(ix)) {
            let (f, m) = (*f, *m);
            self.open(f, m, true, cx);
        }
    }

    fn set_results(&mut self, results: Vec<FileResult>, status: Status, cx: &mut Context<Self>) {
        self.rows = results
            .iter()
            .enumerate()
            .flat_map(|(f, file)| {
                std::iter::once(Row::File(f)).chain((0..file.matches.len()).map(move |m| Row::Match(f, m)))
            })
            .collect();
        self.results = results;
        self.status = status;
        cx.notify();
    }

    fn open(&mut self, file: usize, line_match: usize, keep_focus: bool, cx: &mut Context<Self>) {
        let file = &self.results[file];
        let m = &file.matches[line_match];
        cx.emit(ProjectSearchEvent::Open {
            path: file.path.clone(),
            line: m.line,
            columns: m.columns.clone(),
            query: self.query.clone(),
            keep_focus,
        });
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Theme>();
        let row = div()
            .id(ix)
            .h(px(ROW_HEIGHT))
            .mx(px(6.))
            .rounded(px(ui::R_ROW))
            .flex()
            .items_center()
            .gap(px(8.))
            .pr(px(6.))
            .whitespace_nowrap()
            .cursor_pointer()
            .when(self.selected == Some(ix), |r| r.bg(theme.accent_soft))
            .hover(|s| s.bg(theme.hairline));
        match self.rows[ix] {
            Row::File(f) => {
                let file = &self.results[f];
                let name_start = file.relative.rfind(['/', '\\']).map_or(0, |i| i + 1);
                let folder = file.relative[..name_start].trim_end_matches(['/', '\\']).to_string();
                row.pl(px(10.))
                    .text_size(px(ui::T_SM))
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.foreground)
                            .font_weight(FontWeight::MEDIUM)
                            .child(file.relative[name_start..].to_string()),
                    )
                    .child(div().flex_1().min_w_0().truncate().text_color(theme.muted).child(folder))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(ui::T_XS))
                            .text_color(theme.muted)
                            .child(file.matches.len().to_string()),
                    )
                    .active(|s| s.opacity(0.7))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open(f, 0, false, cx)))
                    .into_any_element()
            }
            Row::Match(f, m) => {
                let line_match = &self.results[f].matches[m];
                let highlight = HighlightStyle {
                    color: Some(theme.foreground),
                    background_color: Some(theme.selection),
                    ..Default::default()
                };
                let text = if self.show_replace {
                    // What replacing would do: the match struck through, the new text after it.
                    let replacement = self.replacement(cx);
                    let (shown, marks) = replace_preview(
                        &line_match.preview,
                        &line_match.highlights,
                        &replacement,
                        self.compiled.as_ref().filter(|_| self.query.regex),
                    );
                    let old = HighlightStyle {
                        color: Some(theme.faint),
                        strikethrough: Some(gpui::StrikethroughStyle { thickness: px(1.), color: Some(theme.faint) }),
                        ..Default::default()
                    };
                    let new = HighlightStyle {
                        color: Some(theme.foreground),
                        background_color: Some(theme.accent_soft),
                        ..Default::default()
                    };
                    StyledText::new(shown)
                        .with_highlights(marks.into_iter().map(|(r, is_new)| (r, if is_new { new } else { old })))
                } else {
                    StyledText::new(line_match.preview.clone())
                        .with_highlights(line_match.highlights.iter().map(|r| (r.clone(), highlight)))
                };
                let group: SharedString = format!("match-{ix}").into();
                row.pl(px(10.))
                    .group(group.clone())
                    .text_size(px(ui::T_SM))
                    .text_color(theme.muted)
                    .child(
                        div()
                            .w(px(32.))
                            .flex_none()
                            .text_right()
                            .text_color(theme.faint)
                            .child((line_match.line + 1).to_string()),
                    )
                    .child(div().flex_1().min_w_0().overflow_hidden().code_font(cx).child(text))
                    .when(self.show_replace, |row| {
                        row.child(
                            div()
                                .id(("replace-line", ix))
                                .flex_none()
                                .px(px(6.))
                                .rounded(px(ui::R_KEY))
                                .text_size(px(ui::T_XS))
                                .text_color(theme.muted)
                                .invisible()
                                .group_hover(group, |s| s.visible())
                                .hover(|s| s.bg(theme.hairline).text_color(theme.foreground))
                                .child("Replace")
                                .active(|s| s.opacity(0.7))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.replace_line(f, m, cx)
                                })),
                        )
                    })
                    .active(|s| s.opacity(0.7))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open(f, m, false, cx)))
                    .into_any_element()
            }
        }
    }

    fn toggle(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        flip: fn(&mut Self),
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.global::<Theme>();
        let tip = match id {
            "case" => "Match case",
            "word" => "Whole word",
            _ => "Regular expression",
        };
        div()
            .id(id)
            .tooltip(ui::tip(tip, None))
            .h(px(22.))
            .min_w(px(24.))
            .px(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(ui::R_KEY))
            .code_font(cx)
            .text_size(px(ui::T_SM))
            .text_color(if on { theme.caret } else { theme.muted })
            .when(on, |b| b.bg(theme.accent_soft))
            .when(!on, |b| b.hover(|s| s.bg(theme.hairline).text_color(theme.foreground)))
            .child(label)
            .active(|s| s.opacity(0.7))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                flip(this);
                this.search(cx);
            }))
    }
}

/// Every match of `query` in the project's files, respecting `.gitignore`.
/// Returns whether the search stopped early at [`MAX_MATCHES`].
/// Every match of `query` in the project's files, respecting `.gitignore`, reported to
/// `report` in batches, files in order, then `Found::Done`. Stops quietly when `cancel` is set.
fn search_files(
    root: &Path,
    query: &SearchQuery,
    files_wanted: &str,
    unsaved: &std::collections::HashMap<PathBuf, ropey::Rope>,
    cancel: &AtomicBool,
    mut report: impl FnMut(Found),
) {
    let Ok(regex) = query.build() else { return };
    let mut walk = ignore::WalkBuilder::new(root);
    if let Some(only) = file_filter(root, files_wanted) {
        walk.overrides(only);
    }
    let mut files: Vec<PathBuf> = walk
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_FILE_SIZE))
        .map(|e| e.into_path())
        .collect();
    files.sort();

    let mut batch = Vec::new();
    let mut sent = Instant::now();
    let mut total = 0;
    for path in files {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let text = match unsaved.get(&path) {
            Some(rope) => rope.to_string(),
            // In its own encoding, so an old Latin-1 file is searched too.
            None => match crate::encoding::read(&path) {
                Ok((text, _)) => text,
                Err(_) => continue,
            },
        };
        if text.contains('\0') {
            continue;
        }
        let mut matches = Vec::new();
        for (line, line_text) in text.lines().enumerate() {
            let found: Vec<Range<usize>> =
                regex.find_iter(line_text).filter(|m| !m.is_empty()).map(|m| m.range()).collect();
            let Some(first) = found.first() else { continue };
            let columns = line_text[..first.start].chars().count()..line_text[..first.end].chars().count();
            let (preview, highlights) = preview(line_text, &found);
            matches.push(LineMatch { line, columns, preview, highlights });
            total += 1;
            if total >= MAX_MATCHES {
                break;
            }
        }
        if !matches.is_empty() {
            let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
            batch.push(FileResult { path, relative, matches });
        }
        if total >= MAX_MATCHES {
            report(Found::Files(batch));
            return report(Found::Done { truncated: true });
        }
        if !batch.is_empty() && sent.elapsed() >= BATCH_EVERY {
            report(Found::Files(std::mem::take(&mut batch)));
            sent = Instant::now();
        }
    }
    if !batch.is_empty() {
        report(Found::Files(batch));
    }
    report(Found::Done { truncated: false });
}

/// Which files to search, from what's written in the files field: names and patterns
/// (`*.rs`, `main.go`), folders (`src/`, `tests`), any of them; `!` before one leaves those
/// out. None when it says nothing (every file).
fn file_filter(root: &Path, written: &str) -> Option<ignore::overrides::Override> {
    let mut only = ignore::overrides::OverrideBuilder::new(root);
    let mut any = false;
    for term in written.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let (leave_out, term) = match term.strip_prefix('!') {
            Some(rest) => (true, rest.trim()),
            None => (false, term),
        };
        let name = term.trim_matches('/');
        if name.is_empty() {
            continue;
        }
        let globs = if term.contains(['*', '?', '[']) {
            vec![term.to_string()]
        } else if term.ends_with('/') {
            // A folder, wherever it is.
            vec![format!("**/{name}/**")]
        } else {
            // A file's name or a folder's (`Makefile`, `.github`, `main.rs`, `src`).
            vec![format!("**/{name}"), format!("**/{name}/**")]
        };
        for glob in globs {
            any |= only.add(&if leave_out { format!("!{glob}") } else { glob }).is_ok();
        }
    }
    any.then(|| only.build().ok()).flatten()
}

/// A match's replacement: `$1`-style groups filled in for a regex search.
pub fn expand(regex: Option<&regex::Regex>, matched: &str, replacement: &str) -> String {
    match regex.and_then(|r| r.captures(matched)) {
        Some(caps) => {
            let mut out = String::new();
            caps.expand(replacement, &mut out);
            out
        }
        None => replacement.to_string(),
    }
}

/// The replacements to make in `text`: byte ranges and their new text, first to last,
/// only on line `only_line` (zero-based) when given.
pub fn replacements(
    text: &str,
    query: &SearchQuery,
    replacement: &str,
    only_line: Option<usize>,
) -> Vec<(Range<usize>, String)> {
    let Ok(regex) = query.build() else { return Vec::new() };
    let span = match only_line {
        Some(line) => {
            let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
            let len = text[start.min(text.len())..].split_inclusive('\n').next().map_or(0, str::len);
            start..start + len
        }
        None => 0..text.len(),
    };
    regex
        .captures_iter(text)
        .filter_map(|caps| {
            let m = caps.get(0)?;
            if m.is_empty() || m.start() < span.start || m.end() > span.end {
                return None;
            }
            let new = if query.regex {
                let mut out = String::new();
                caps.expand(replacement, &mut out);
                out
            } else {
                replacement.to_string()
            };
            Some((m.range(), new))
        })
        .collect()
}

/// A preview line as a replace would leave it: each match kept (to be struck through)
/// with its replacement after it. Returns the text and its marks (range, is the new text).
fn replace_preview(
    preview: &str,
    matches: &[Range<usize>],
    replacement: &str,
    regex: Option<&regex::Regex>,
) -> (SharedString, Vec<(Range<usize>, bool)>) {
    let mut shown = String::new();
    let mut marks = Vec::new();
    let mut at = 0;
    for m in matches {
        shown.push_str(&preview[at..m.start]);
        let old = &preview[m.clone()];
        marks.push((shown.len()..shown.len() + old.len(), false));
        shown.push_str(old);
        let new = expand(regex, old, replacement);
        if !new.is_empty() {
            marks.push((shown.len()..shown.len() + new.len(), true));
            shown.push_str(&new);
        }
        at = m.end;
    }
    shown.push_str(&preview[at..]);
    (shown.into(), marks)
}

/// A trimmed, length-limited version of a line, with match ranges shifted to fit it.
fn preview(line: &str, matches: &[Range<usize>]) -> (SharedString, Vec<Range<usize>>) {
    let indent = line.len() - line.trim_start().len();
    let first = matches[0].start;
    // Keep some context before the first match on long lines.
    let mut start = indent.max(first.saturating_sub(40));
    while !line.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = line[start..].char_indices().nth(PREVIEW_CHARS).map_or(line.len(), |(i, _)| start + i);
    end = end.max(start);
    let preview = line[start..end].trim_end();
    // Highlights stay inside the preview, which lost its trailing spaces.
    let shifted = matches
        .iter()
        .filter(|m| m.start >= start && m.end <= end)
        .map(|m| (m.start - start).min(preview.len())..(m.end - start).min(preview.len()))
        .filter(|r| !r.is_empty())
        .collect();
    (preview.to_string().into(), shifted)
}

impl Focusable for ProjectSearch {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for ProjectSearch {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let case = self.toggle("case", "Aa", self.case_sensitive, |s| s.case_sensitive = !s.case_sensitive, cx);
        let word = self.toggle("word", "W", self.whole_word, |s| s.whole_word = !s.whole_word, cx);
        let regex = self.toggle("regex", ".*", self.regex, |s| s.regex = !s.regex, cx);
        let theme = cx.global::<Theme>();
        let status: SharedString = match self.status {
            Status::Idle => "".into(),
            Status::Searching => "Searching…".into(),
            Status::Invalid => "Invalid pattern".into(),
            Status::Done { matches: 0, .. } => "No results".into(),
            Status::Done { files, matches, truncated } => {
                let plus = if truncated { "+" } else { "" };
                let files = if files == 1 { "1 file".to_string() } else { format!("{files} files") };
                let results = if matches == 1 { "result" } else { "results" };
                format!("{matches}{plus} {results} in {files}").into()
            }
        };
        let invalid = matches!(self.status, Status::Invalid);
        div()
            .key_context("ProjectSearch")
            .on_action(cx.listener(Self::next_result))
            .on_action(cx.listener(Self::previous_result))
            .on_action(cx.listener(Self::open_result))
            .on_action(cx.listener(Self::replace_all))
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .mx(px(12.))
                    .h(px(ui::FIELD))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .rounded(px(ui::R_ROW))
                    .bg(theme.background)
                    .border_1()
                    .border_color(if invalid { theme.error } else { theme.hairline })
                    .text_size(px(ui::T_MD))
                    .line_height(px(20.))
                    .child(
                        div()
                            .id("toggle-replace")
                            .flex_none()
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(ui::R_KEY))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.hairline))
                            .child(
                                gpui::svg()
                                    .path("icons/chevron-right.svg")
                                    .size(px(11.))
                                    .text_color(theme.muted)
                                    .with_transformation(gpui::Transformation::rotate(gpui::radians(
                                        if self.show_replace { std::f32::consts::FRAC_PI_2 } else { 0. },
                                    ))),
                            )
                            .tooltip(ui::tip("Replace, and which files", None))
                            .active(|s| s.opacity(0.7))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.show_replace = !this.show_replace;
                                if this.show_replace {
                                    window.focus(&this.replace_input.focus_handle(cx));
                                }
                                // The files field stops (or starts) counting.
                                if !this.files_input.read(cx).text().is_empty() {
                                    this.search(cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1().min_w_0().overflow_hidden().child(self.input.clone()))
                    .child(case)
                    .child(word)
                    .child(regex),
            )
            .when(self.show_replace, |panel| {
                let can_replace = !self.results.is_empty();
                panel
                    .child(
                        div()
                            .mx(px(12.))
                            .mt(px(6.))
                            .flex()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(ui::FIELD))
                                    .px(px(8.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(ui::R_ROW))
                                    .bg(theme.background)
                                    .border_1()
                                    .border_color(theme.hairline)
                                    .text_size(px(ui::T_MD))
                                    .line_height(px(20.))
                                    .overflow_hidden()
                                    .child(self.replace_input.clone()),
                            )
                            .child(
                                div()
                                    .id("replace-all")
                                    .flex_none()
                                    .h(px(ui::FIELD))
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(ui::R_ROW))
                                    .text_size(px(ui::T_SM))
                                    .when(can_replace, |b| {
                                        b.cursor_pointer()
                                            .text_color(theme.foreground)
                                            .hover(|s| s.bg(theme.hairline))
                                            .active(|s| s.opacity(0.7))
                                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                                this.replace_all(&ReplaceAllResults, window, cx)
                                            }))
                                    })
                                    .when(!can_replace, |b| b.text_color(theme.faint))
                                    .child("Replace all"),
                            ),
                    )
                    .child(
                        div()
                            .mx(px(12.))
                            .mt(px(6.))
                            .h(px(ui::FIELD))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .rounded(px(ui::R_ROW))
                            .bg(theme.background)
                            .border_1()
                            .border_color(theme.hairline)
                            .text_size(px(ui::T_MD))
                            .line_height(px(20.))
                            .overflow_hidden()
                            .child(self.files_input.clone()),
                    )
            })
            .child(
                div()
                    .px(px(16.))
                    .py(px(8.))
                    .text_size(px(ui::T_SM))
                    .text_color(if invalid { theme.error } else { theme.muted })
                    .child(status),
            )
            .child(
                uniform_list(
                    "project-search-results",
                    self.rows.len(),
                    cx.processor(|this, range: Range<usize>, _, cx| range.map(|ix| this.render_row(ix, cx)).collect()),
                )
                .track_scroll(self.scroll.clone())
                .flex_1(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacements_fill_in_groups_and_can_keep_to_one_line() {
        let text = "let a = old(1);\nlet b = old(2);\n";
        let plain = SearchQuery { text: "old".into(), ..Default::default() };
        assert_eq!(replacements(text, &plain, "new", None).len(), 2);
        assert_eq!(replacements(text, &plain, "new", Some(1)), vec![(24..27, "new".into())]);
        let regex = SearchQuery { text: r"old\((\d)\)".into(), regex: true, ..Default::default() };
        let found = replacements(text, &regex, "new($1, 0)", None);
        assert_eq!(found[1].1, "new(2, 0)");
        // In a plain search, "$1" is just text.
        assert_eq!(replacements(text, &plain, "$1", Some(0))[0].1, "$1");
    }

    #[test]
    fn the_files_field_picks_which_files_are_searched() {
        let root = std::env::temp_dir().join(format!("null-search-files-{}", std::process::id()));
        for file in ["src/main.rs", "src/ui/view.rs", "src/notes.md", "tests/it.rs", "docs/main.rs"] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "needle\n").unwrap();
        }
        let query = SearchQuery { text: "needle".into(), ..Default::default() };
        let found = |files: &str| {
            let mut found = Vec::new();
            search_files(&root, &query, files, &Default::default(), &AtomicBool::new(false), |f| {
                if let Found::Files(files) = f {
                    found.extend(files.into_iter().map(|f| f.relative));
                }
            });
            found.join(" ")
        };
        assert_eq!(found(""), "docs/main.rs src/main.rs src/notes.md src/ui/view.rs tests/it.rs");
        assert_eq!(found("*.rs, !tests"), "docs/main.rs src/main.rs src/ui/view.rs");
        assert_eq!(found("src/"), "src/main.rs src/notes.md src/ui/view.rs");
        assert_eq!(found("ui"), "src/ui/view.rs");
        assert_eq!(found("main.rs"), "docs/main.rs src/main.rs");
        assert_eq!(found("!src, !docs"), "tests/it.rs");
        // A name without a dot can be a file, and one with a dot a folder.
        assert_eq!(found("it.rs"), "tests/it.rs");
        assert_eq!(found("notes.md, tests"), "src/notes.md tests/it.rs");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_preview_shows_old_and_new_side_by_side() {
        let (shown, marks) = replace_preview("a(old)", &[2..5], "new", None);
        assert_eq!(shown.as_ref(), "a(oldnew)");
        assert_eq!(marks, vec![(2..5, false), (5..8, true)]);
    }

    #[gpui::test]
    fn arrows_step_through_matches_skipping_file_names(cx: &mut gpui::TestAppContext) {
        let search = cx.new(|cx| ProjectSearch::new(PathBuf::from("/p"), cx));
        let m = |line| LineMatch { line, columns: 0..1, preview: "x".into(), highlights: vec![] };
        let file = |name: &str, lines: Vec<usize>| FileResult {
            path: PathBuf::from(name),
            relative: name.into(),
            matches: lines.into_iter().map(m).collect(),
        };
        search.update(cx, |s, cx| {
            s.set_results(vec![file("a", vec![1, 2]), file("b", vec![3])], Status::Idle, cx);
            // Rows: a, a:1, a:2, b, b:3.
            s.step(true, cx);
            assert_eq!(s.selected, Some(1));
            s.step(true, cx);
            s.step(true, cx);
            assert_eq!(s.selected, Some(4));
            // At the end it stays.
            s.step(true, cx);
            assert_eq!(s.selected, Some(4));
            s.step(false, cx);
            assert_eq!(s.selected, Some(2));
        });
    }

    #[test]
    fn finds_the_notes_left_in_code() {
        let dir = std::env::temp_dir().join(format!("null-search-todos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "// TODO: split\nfn a() {} // FIXME later\n// TODOS, todo, XXXL\n").unwrap();
        let mut lines = Vec::new();
        search_files(&dir, &todo_query(), "", &Default::default(), &AtomicBool::new(false), |found| {
            if let Found::Files(files) = found {
                lines.extend(files.into_iter().flat_map(|f| f.matches.into_iter().map(|m| m.line)));
            }
        });
        lines.sort();
        assert_eq!(lines, [0, 1]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn searches_files_in_other_encodings() {
        let dir = std::env::temp_dir().join(format!("null-search-latin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.txt"), b"caf\xe9 cr\xe8me\n").unwrap();
        let query = SearchQuery { text: "crème".into(), ..Default::default() };
        let mut results = Vec::new();
        search_files(&dir, &query, "", &Default::default(), &AtomicBool::new(false), |found| {
            if let Found::Files(files) = found {
                results.extend(files)
            }
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches[0].preview.as_ref(), "café crème");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_matches_across_files_and_skips_ignored_ones() {
        let dir = std::env::temp_dir().join(format!("null-search-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn alpha() {}\n    let beta = alpha();\n").unwrap();
        std::fs::write(dir.join("src/b.rs"), "nothing here\n").unwrap();
        std::fs::write(dir.join("target/c.rs"), "alpha\n").unwrap();

        let query = SearchQuery { text: "alpha".into(), ..Default::default() };
        let mut results = Vec::new();
        let mut truncated = None;
        search_files(&dir, &query, "", &Default::default(), &AtomicBool::new(false), |found| match found {
            Found::Files(files) => results.extend(files),
            Found::Done { truncated: t } => truncated = Some(t),
        });
        let truncated = truncated.expect("a finished search says so");
        assert!(!truncated);
        assert_eq!(results.len(), 1);
        let file = &results[0];
        assert_eq!(file.relative, Path::new("src").join("a.rs").to_string_lossy());
        assert_eq!(file.matches.len(), 2);
        assert_eq!((file.matches[1].line, file.matches[1].columns.clone()), (1, 15..20));
        // The preview drops indentation and keeps the highlight on the match.
        assert_eq!(file.matches[1].preview.as_ref(), "let beta = alpha();");
        assert_eq!(file.matches[1].highlights, vec![11..16]);

        // A cancelled search reports nothing.
        let mut reported = false;
        search_files(&dir, &query, "", &Default::default(), &AtomicBool::new(true), |_| reported = true);
        assert!(!reported);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
