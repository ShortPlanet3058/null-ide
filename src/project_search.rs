use crate::fonts::CodeFont;
use crate::search::SearchQuery;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use crate::ui;
use crate::ui::px;
use futures::StreamExt;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle, KeyBinding,
    ScrollStrategy, SharedString, StyledText, Subscription, Task, UniformListScrollHandle, Window, actions, div,
    prelude::*, uniform_list,
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
    /// Where the first match ends, when on a later line (line, char column).
    pub end: Option<(usize, usize)>,
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
    /// Some of its matches were taken off the list: replacing goes line by line.
    dismissed: bool,
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
    /// `end`: where the match ends, when on a later line.
    Open {
        path: PathBuf,
        line: usize,
        columns: Range<usize>,
        end: Option<(usize, usize)>,
        query: SearchQuery,
        keep_focus: bool,
    },
    /// Replace the matches in these files (only on the given line, when there is one).
    Replace { query: SearchQuery, replacement: String, targets: Vec<(PathBuf, Option<Vec<usize>>)> },
}

/// Search across every file in the project. Lives in the sidebar.
pub struct ProjectSearch {
    /// The window's folders, searched together.
    roots: crate::roots::ProjectRoots,
    input: Entity<TextInput>,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    /// Only the files open in tabs.
    open_only: bool,
    /// The files open in this window's tabs: kept up to date by its workspace.
    pub open_files: Vec<PathBuf>,
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
    /// Results taken off the list (a file's line, or the whole file), for this query: they
    /// stay off when it runs again (after replacing).
    dismissed: (SearchQuery, std::collections::HashSet<(PathBuf, Option<usize>)>),
    /// Searches used lately (a result opened, or replaced), most recent first: shown while
    /// the field is empty, to search again.
    pub history: Vec<SearchQuery>,
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
            roots: crate::roots::ProjectRoots::new(root),
            input,
            case_sensitive: false,
            whole_word: false,
            open_only: false,
            open_files: Vec::new(),
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
            dismissed: Default::default(),
            history: Vec::new(),
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

    /// Searches only in `folder` (as written from the project's root): the files field opens
    /// with it, the search field focused.
    pub fn search_in_folder(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.show_replace = true;
        // From the root, as written: not every folder of that name, nor a pattern.
        self.files_input.update(cx, |input, cx| input.set_text(&format!("/{folder}/"), cx));
        self.focus(None, window, cx);
        cx.notify();
    }

    /// Runs the search again (after a replace changed the files).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.search(cx);
    }

    /// The search now, used (a result opened, or replaced): first among the recent ones.
    fn remember(&mut self) {
        const KEPT: usize = 10;
        let query = self.query.clone();
        if query.text.trim().is_empty() {
            return;
        }
        self.history.retain(|q| q != &query);
        self.history.insert(0, query);
        self.history.truncate(KEPT);
    }

    fn replacement(&self, cx: &App) -> String {
        self.replace_input.read(cx).text().to_string()
    }

    /// ⌘↵ or the button: every match in every file found.
    fn replace_all(&mut self, _: &ReplaceAllResults, _: &mut Window, cx: &mut Context<Self>) {
        if !self.show_replace || self.results.is_empty() {
            return;
        }
        let targets = self.results.iter().flat_map(replace_targets).collect();
        self.remember();
        cx.emit(ProjectSearchEvent::Replace { query: self.query.clone(), replacement: self.replacement(cx), targets });
    }

    /// `file` without what was taken off the list (none of it, when it all was).
    fn undismissed(&self, mut file: FileResult) -> Option<FileResult> {
        let gone = &self.dismissed.1;
        if gone.is_empty() {
            return Some(file);
        }
        if gone.contains(&(file.path.clone(), None)) {
            return None;
        }
        let before = file.matches.len();
        file.matches.retain(|m| !gone.contains(&(file.path.clone(), Some(m.line))));
        file.dismissed = file.matches.len() != before;
        (!file.matches.is_empty()).then_some(file)
    }

    /// The button on a file's row: its matches, in that file only.
    fn replace_file(&mut self, file: usize, cx: &mut Context<Self>) {
        let targets = replace_targets(&self.results[file]);
        self.remember();
        cx.emit(ProjectSearchEvent::Replace { query: self.query.clone(), replacement: self.replacement(cx), targets });
    }

    /// The ✕ on a result: off the list (not replaced by Replace All); on a file, all of it.
    fn dismiss(&mut self, file: usize, line_match: Option<usize>, cx: &mut Context<Self>) {
        let path = self.results[file].path.clone();
        let line = line_match.map(|m| self.results[file].matches[m].line);
        self.dismissed.1.insert((path, line));
        match line_match {
            Some(m) => {
                let result = &mut self.results[file];
                result.matches.remove(m);
                result.dismissed = true;
                if result.matches.is_empty() {
                    self.results.remove(file);
                }
            }
            None => {
                self.results.remove(file);
            }
        }
        let results = std::mem::take(&mut self.results);
        let status = match self.status {
            Status::Done { truncated, .. } => {
                Status::Done { files: results.len(), matches: results.iter().map(|f| f.matches.len()).sum(), truncated }
            }
            Status::Idle => Status::Idle,
            Status::Searching => Status::Searching,
            Status::Invalid => Status::Invalid,
        };
        self.set_results(results, status, cx);
        // The keyboard's place: on a result still listed, near where it was.
        self.selected = self.selected.and_then(|s| {
            let last = self.rows.len().checked_sub(1)?;
            let s = s.min(last);
            (s..=last).chain((0..s).rev()).find(|&i| matches!(self.rows[i], Row::Match(..)))
        });
    }

    fn replace_line(&mut self, file: usize, line_match: usize, cx: &mut Context<Self>) {
        self.remember();
        let file = &self.results[file];
        let line = file.matches[line_match].line;
        cx.emit(ProjectSearchEvent::Replace {
            query: self.query.clone(),
            replacement: self.replacement(cx),
            targets: vec![(file.path.clone(), Some(vec![line]))],
        });
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.set_roots(crate::roots::ProjectRoots::new(root), cx);
    }

    pub fn set_roots(&mut self, roots: crate::roots::ProjectRoots, cx: &mut Context<Self>) {
        self.roots = roots;
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
        // New text searches by itself (the field says it changed); the same text, here.
        let same = self.input.read(cx).text() == query.text;
        self.input.update(cx, |input, cx| input.set_text(&query.text, cx));
        window.focus(&self.input.focus_handle(cx));
        if same {
            self.search(cx);
        }
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
        if self.dismissed.0 != query {
            self.dismissed = (query.clone(), Default::default());
        }
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
        let roots = self.roots.clone();
        // The files field counts only while it shows.
        let files = if self.show_replace { self.files_input.read(cx).text().to_string() } else { String::new() };
        let unsaved = cx.try_global::<UnsavedFiles>().map(|u| u.0.clone()).unwrap_or_default();
        let hide = cx.global::<crate::settings::Settings>().hidden_paths();
        let only: Option<std::collections::HashSet<PathBuf>> =
            self.open_only.then(|| self.open_files.iter().cloned().collect());
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        // Results show as they're found, file by file in order; the previous results stay
        // until the first new ones arrive, so the list doesn't flash empty while typing.
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let (tx, mut rx) = futures::channel::mpsc::unbounded();
            cx.background_executor()
                .spawn(async move {
                    search_files(&roots, &query, &files, &hide, only.as_ref(), &unsaved, &cancel, |found| {
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
                            results.extend(files.into_iter().filter_map(|file| this.undismissed(file)));
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

    /// The searches used lately, while the field is empty: a click searches again.
    fn render_history(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let rows =
            self.history.iter().enumerate().map(|(ix, query)| {
                let query = query.clone();
                // What it searched with, after it: Aa, W, .*
                let how: Vec<&str> = [(query.case_sensitive, "Aa"), (query.whole_word, "W"), (query.regex, ".*")]
                    .into_iter()
                    .filter_map(|(on, mark)| on.then_some(mark))
                    .collect();
                div()
                    .id(("search-history", ix))
                    .mx(px(6.))
                    .h(px(ui::ROW))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(ui::R_ROW))
                    .cursor_pointer()
                    .hover(|s| s.bg(ui::hover(&theme)))
                    .text_size(px(ui::T_MD))
                    .text_color(theme.muted)
                    .child(div().min_w_0().truncate().code_font(cx).child(query.text.clone()))
                    .children((!how.is_empty()).then(|| {
                        div().flex_none().text_size(px(ui::T_SM)).text_color(theme.faint).child(how.join(" "))
                    }))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, window, cx| this.search_for(query.clone(), window, cx)),
                    )
            });
        div()
            .flex()
            .flex_col()
            .child(div().px(px(16.)).pb(px(4.)).text_size(px(ui::T_SM)).text_color(theme.faint).child("Recent"))
            .children(rows)
    }

    fn open(&mut self, file: usize, line_match: usize, keep_focus: bool, cx: &mut Context<Self>) {
        self.remember();
        let file = &self.results[file];
        let m = &file.matches[line_match];
        cx.emit(ProjectSearchEvent::Open {
            path: file.path.clone(),
            line: m.line,
            columns: m.columns.clone(),
            end: m.end,
            query: self.query.clone(),
            keep_focus,
        });
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.global::<Theme>();
        let row = div()
            .id(ix)
            .h(px(ROW_HEIGHT))
            .w_full()
            .rounded(px(ui::R_ROW))
            .flex()
            .items_center()
            .gap(px(8.))
            .pr(px(6.))
            .whitespace_nowrap()
            .cursor_pointer()
            .when(self.selected == Some(ix), |r| r.bg(theme.hairline))
            .when(self.selected != Some(ix), |r| r.hover(|s| s.bg(ui::hover(theme))));
        match self.rows[ix] {
            Row::File(f) => {
                let group: SharedString = format!("file-{ix}").into();
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
                    .group(group.clone())
                    .when(self.show_replace, |row| {
                        row.child(self.row_button(("replace-file", ix), "Replace", group.clone(), cx).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.replace_file(f, cx)
                            }),
                        ))
                    })
                    .child(self.dismiss_button(("dismiss-file", ix), group, cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.dismiss(f, None, cx)
                        },
                    )))
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
                        self.query.keeps_case(&replacement),
                    );
                    let old = HighlightStyle {
                        color: Some(theme.faint),
                        strikethrough: Some(gpui::StrikethroughStyle {
                            thickness: gpui::px(1.),
                            color: Some(theme.faint),
                        }),
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
                        row.child(self.row_button(("replace-line", ix), "Replace", group.clone(), cx).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.replace_line(f, m, cx)
                            }),
                        ))
                    })
                    .child(self.dismiss_button(("dismiss-match", ix), group, cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.dismiss(f, Some(m), cx)
                        },
                    )))
                    .active(|s| s.opacity(0.7))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open(f, m, false, cx)))
                    .into_any_element()
            }
        }
    }

    /// The ✕ on a result row, shown while the row is hovered.
    fn dismiss_button(&self, id: (&'static str, usize), group: SharedString, cx: &App) -> gpui::Stateful<gpui::Div> {
        ui::icon_button(id, "icons/x.svg", "Remove from results", cx.global::<Theme>())
            .size(px(18.))
            .invisible()
            .group_hover(group, |s| s.visible())
    }

    /// A small button on a result row, shown while the row is hovered.
    fn row_button(
        &self,
        id: (&'static str, usize),
        label: &'static str,
        group: SharedString,
        cx: &App,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = cx.global::<Theme>();
        div()
            .id(id)
            .flex_none()
            .px(px(6.))
            .rounded(px(ui::R_KEY))
            .text_size(px(ui::T_XS))
            .text_color(theme.muted)
            .invisible()
            .group_hover(group, |s| s.visible())
            .hover(|s| s.bg(ui::hover(theme)).text_color(theme.foreground))
            .child(label)
            .active(|s| s.opacity(0.7))
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
            "open" => "Only the files open in tabs",
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
            .when(!on, |b| b.hover(|s| s.bg(ui::hover(theme)).text_color(theme.foreground)))
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
#[allow(clippy::too_many_arguments)]
fn search_files(
    roots: &crate::roots::ProjectRoots,
    query: &SearchQuery,
    files_wanted: &str,
    hide: &[String],
    only: Option<&std::collections::HashSet<PathBuf>>,
    unsaved: &std::collections::HashMap<PathBuf, ropey::Rope>,
    cancel: &AtomicBool,
    report: impl FnMut(Found),
) {
    let Ok(regex) = query.build() else { return };
    let mut files: Vec<PathBuf> = match only {
        // The open files themselves: ignored by git (`.env`) or outside the project too.
        Some(only) => {
            let wanted = file_filter(roots.primary(), files_wanted);
            only.iter()
                .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE_SIZE))
                .filter(|p| wanted.as_ref().is_none_or(|w| !w.matched(p, false).is_ignore()))
                .cloned()
                .collect()
        }
        // Each folder of the window, `/folder/` in the files field from its own top.
        None => roots.all().iter().flat_map(|root| files_in(root, files_wanted, hide, cancel)).collect(),
    };
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    files.sort();
    search_in(roots, &files, &regex, query, unsaved, cancel, report);
}

/// The files of `root` a search looks in: `.gitignore`, the `hide` setting and the files
/// field respected, read side by side.
fn files_in(root: &Path, files_wanted: &str, hide: &[String], cancel: &AtomicBool) -> Vec<PathBuf> {
    let wanted = file_filter(root, files_wanted);
    {
        {
            use ignore::WalkState;
            let mut walk = ignore::WalkBuilder::new(root);
            if let Some(wanted) = wanted {
                walk.overrides(wanted);
            }
            // (What the `hide` setting keeps out, folders and all.)
            let hidden = crate::project_index::hidden_rules(root, hide);
            // Folders read side by side.
            let found: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
            walk.hidden(false)
                .filter_entry(move |e| {
                    e.file_name() != ".git"
                        && !hidden.matched(e.path(), e.file_type().is_some_and(|t| t.is_dir())).is_ignore()
                })
                .build_parallel()
                .run(|| {
                    let found = &found;
                    Box::new(move |entry| {
                        if cancel.load(Ordering::Relaxed) {
                            return WalkState::Quit;
                        }
                        let Ok(entry) = entry else { return WalkState::Continue };
                        let wanted = entry.file_type().is_some_and(|t| t.is_file())
                            && entry.metadata().is_ok_and(|m| m.len() <= MAX_FILE_SIZE);
                        if wanted && let Ok(mut found) = found.lock() {
                            found.push(entry.into_path());
                        }
                        WalkState::Continue
                    })
                });
            found.into_inner().unwrap_or_default()
        }
    }
}

/// `files` searched for `regex`, reported in batches, in order.
fn search_in(
    roots: &crate::roots::ProjectRoots,
    files: &[PathBuf],
    regex: &regex::Regex,
    query: &SearchQuery,
    unsaved: &std::collections::HashMap<PathBuf, ropey::Rope>,
    cancel: &AtomicBool,
    mut report: impl FnMut(Found),
) {
    // Files read and searched on every core, a window of them at a time, then reported in
    // order (one by one, a project of 42,000 files took 11 s).
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let window = cores * 32;
    let mut batch = Vec::new();
    let mut sent = Instant::now();
    let mut total = 0;
    for paths in files.chunks(window) {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let most = MAX_MATCHES - total;
        let per_core = paths.len().div_ceil(cores);
        let searched: Vec<Vec<LineMatch>> = std::thread::scope(|s| {
            let parts: Vec<_> = paths
                .chunks(per_core)
                .map(|part| {
                    let (regex, query) = (regex, query);
                    s.spawn(move || {
                        part.iter()
                            .map(|path| matches_in(path, unsaved, regex, query, most, cancel))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            // (A part that failed: none in its files, each still in its place.)
            parts
                .into_iter()
                .zip(paths.chunks(per_core))
                .flat_map(|(part, its)| part.join().unwrap_or_else(|_| its.iter().map(|_| Vec::new()).collect()))
                .collect()
        });
        for (path, mut matches) in paths.iter().zip(searched) {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            matches.truncate(MAX_MATCHES - total);
            total += matches.len();
            if !matches.is_empty() {
                let relative = roots.display(path);
                batch.push(FileResult { path: path.clone(), relative, matches, dismissed: false });
            }
            if total >= MAX_MATCHES {
                report(Found::Files(batch));
                return report(Found::Done { truncated: true });
            }
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

/// A file's matches of `regex` (at most `most`): the file as it's being written if it's open
/// with changes (`unsaved`), else as it is on disk.
fn matches_in(
    path: &Path,
    unsaved: &std::collections::HashMap<PathBuf, ropey::Rope>,
    regex: &regex::Regex,
    query: &SearchQuery,
    most: usize,
    cancel: &AtomicBool,
) -> Vec<LineMatch> {
    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }
    let text = match unsaved.get(path) {
        Some(rope) => rope.to_string(),
        // In its own encoding, so an old Latin-1 file is searched too; a notebook, as
        // its cells (as it's written in: its lines are those).
        None => match crate::encoding::read(path) {
            Ok((text, _)) => crate::notebook::cells_of_file(path, text),
            Err(_) => return Vec::new(),
        },
    };
    if text.contains('\0') {
        return Vec::new();
    }
    if query.spans_lines() {
        return spanning_matches(&text, regex, most);
    }
    // Most files have none: one look through the whole text says so (line by line was most
    // of the time a search took).
    if !regex.is_match(&text) {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for (line, line_text) in text.lines().enumerate() {
        let found: Vec<Range<usize>> =
            regex.find_iter(line_text).filter(|m| !m.is_empty()).map(|m| m.range()).collect();
        let Some(first) = found.first() else { continue };
        let columns = line_text[..first.start].chars().count()..line_text[..first.end].chars().count();
        let (preview, highlights) = preview(line_text, &found);
        matches.push(LineMatch { line, columns, end: None, preview, highlights });
        if matches.len() >= most {
            break;
        }
    }
    matches
}

/// A line's matches (of a pattern across lines): the line, their ranges on it, and where its
/// first one ends.
type StartingHere = (usize, Vec<Range<usize>>, Option<(usize, usize)>);

/// The matches of a pattern that can cross lines, in the whole `text`: listed by the line
/// each starts on (at most `most` lines), each shown on it as far as it goes there.
fn spanning_matches(text: &str, regex: &regex::Regex, most: usize) -> Vec<LineMatch> {
    let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let line_of = |offset: usize| starts.partition_point(|&s| s <= offset) - 1;
    // Each line's matches: their ranges on it, and where its first one ends.
    let mut lines: Vec<StartingHere> = Vec::new();
    for m in regex.find_iter(text).filter(|m| !m.is_empty()) {
        let line = line_of(m.start());
        let line_start = starts[line];
        let line_text = text[line_start..].split('\n').next().unwrap_or("");
        let line_text = line_text.strip_suffix('\r').unwrap_or(line_text);
        // (Starting on a line's own break, `\r\n`'s `\n`: at its end.)
        let from = (m.start() - line_start).min(line_text.len());
        let on_line = from..(m.end() - line_start).min(line_text.len()).max(from);
        // (Ending just after a line break: at the end of that line, not the next one's start.)
        let end_line = line_of(m.end().saturating_sub(1)).max(line);
        let end = (end_line > line)
            .then(|| (end_line, text[starts[end_line]..m.end()].trim_end_matches(['\n', '\r']).chars().count()));
        match lines.last_mut() {
            Some((l, ranges, _)) if *l == line => ranges.push(on_line),
            _ => {
                if lines.len() >= most {
                    break;
                }
                lines.push((line, vec![on_line], end));
            }
        }
    }
    lines
        .into_iter()
        .map(|(line, ranges, end)| {
            let line_text = text[starts[line]..].split('\n').next().unwrap_or("");
            let line_text = line_text.strip_suffix('\r').unwrap_or(line_text);
            let first = &ranges[0];
            let columns = line_text[..first.start].chars().count()..line_text[..first.end].chars().count();
            let (preview, highlights) = preview(line_text, &ranges);
            LineMatch { line, columns, end, preview, highlights }
        })
        .collect()
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
        let globs = if term.starts_with('/') {
            // From the project's root, exactly as written (Find in Folder writes it so):
            // `[id]` is a folder's name there, not a pattern.
            let exact: String =
                name.chars().flat_map(|c| if "*?[]{}\\!".contains(c) { vec!['\\', c] } else { vec![c] }).collect();
            vec![format!("/{exact}"), format!("/{exact}/**")]
        } else if term.contains(['*', '?', '[']) {
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

/// What replacing in `file`'s results changes: the whole file, or with some taken off the
/// list, the lines still on it.
fn replace_targets(file: &FileResult) -> Vec<(PathBuf, Option<Vec<usize>>)> {
    if !file.dismissed {
        return vec![(file.path.clone(), None)];
    }
    let mut lines: Vec<usize> = file.matches.iter().map(|m| m.line).collect();
    lines.dedup();
    vec![(file.path.clone(), Some(lines))]
}

/// A match's replacement: `$1`-style groups filled in for a regex search, in the match's
/// case when `keep_case` (see `SearchQuery::keeps_case`).
pub fn expand(regex: Option<&regex::Regex>, matched: &str, replacement: &str, keep_case: bool) -> String {
    match regex.and_then(|r| r.captures(matched)) {
        Some(caps) => crate::search::expand_cased(&caps, replacement),
        None if keep_case => crate::search::in_case_of(matched, replacement),
        None => replacement.to_string(),
    }
}

/// The replacements to make in `text`: byte ranges and their new text, first to last,
/// only on lines `only_lines` (zero-based) when given. Line by line, as the search found
/// them: what the preview showed is what changes (a regex like `\s+$` never takes a line
/// break, which would join lines or drop the blank ones).
pub fn replacements(
    text: &str,
    query: &SearchQuery,
    replacement: &str,
    only_lines: Option<&[usize]>,
) -> Vec<(Range<usize>, String)> {
    let Ok(regex) = query.build() else { return Vec::new() };
    let mut out = Vec::new();
    // Across lines: in the whole text, each match kept to the line it starts on.
    if query.spans_lines() {
        // The line counted on as the matches go (not from the start for each).
        let (mut line, mut counted) = (0, 0);
        for caps in regex.captures_iter(text) {
            let Some(m) = caps.get(0).filter(|m| !m.is_empty()) else { continue };
            line += text[counted..m.start()].matches('\n').count();
            counted = m.start();
            if only_lines.is_some_and(|only| !only.contains(&line)) {
                continue;
            }
            out.push((m.range(), crate::search::expand_cased(&caps, replacement)));
        }
        return out;
    }
    let mut line_start = 0;
    for (n, piece) in text.split_inclusive('\n').enumerate() {
        // The line as `str::lines` gives it to the search: without its break.
        let line = piece.strip_suffix('\n').map_or(piece, |l| l.strip_suffix('\r').unwrap_or(l));
        if only_lines.is_none_or(|only| only.contains(&n)) {
            for caps in regex.captures_iter(line) {
                let Some(m) = caps.get(0).filter(|m| !m.is_empty()) else { continue };
                let new = if query.regex {
                    crate::search::expand_cased(&caps, replacement)
                } else if query.keeps_case(replacement) {
                    crate::search::in_case_of(m.as_str(), replacement)
                } else {
                    replacement.to_string()
                };
                out.push((line_start + m.start()..line_start + m.end(), new));
            }
        }
        line_start += piece.len();
    }
    out
}

/// A preview line as a replace would leave it: each match kept (to be struck through)
/// with its replacement after it. Returns the text and its marks (range, is the new text).
fn replace_preview(
    preview: &str,
    matches: &[Range<usize>],
    replacement: &str,
    regex: Option<&regex::Regex>,
    keep_case: bool,
) -> (SharedString, Vec<(Range<usize>, bool)>) {
    let mut shown = String::new();
    let mut marks = Vec::new();
    let mut at = 0;
    for m in matches {
        shown.push_str(&preview[at..m.start]);
        let old = &preview[m.clone()];
        marks.push((shown.len()..shown.len() + old.len(), false));
        shown.push_str(old);
        let new = expand(regex, old, replacement, keep_case);
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
        let open = self.toggle("open", "Tabs", self.open_only, |s| s.open_only = !s.open_only, cx);
        let theme = cx.global::<Theme>();
        let status: SharedString = match self.status {
            Status::Idle => "".into(),
            Status::Searching => "Searching…".into(),
            Status::Invalid => "Invalid pattern".into(),
            Status::Done { matches: 0, .. } => "No matches".into(),
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
                            .hover(|s| s.bg(ui::hover(theme)))
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
                    .child(regex)
                    .child(open),
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
                                            .hover(|s| s.bg(ui::hover(theme)))
                                            .active(|s| s.opacity(0.7))
                                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                                this.replace_all(&ReplaceAllResults, window, cx)
                                            }))
                                    })
                                    .when(!can_replace, |b| b.text_color(theme.faint))
                                    .child("Replace All"),
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
            .when(self.input.read(cx).text().is_empty() && !self.history.is_empty(), |panel| {
                panel.child(self.render_history(cx))
            })
            .child(
                uniform_list(
                    "project-search-results",
                    self.rows.len(),
                    // Each row the panel's width, inset so its highlight has rounded ends.
                    cx.processor(|this, range: Range<usize>, _, cx| {
                        range
                            .map(|ix| div().w_full().px(px(6.)).child(this.render_row(ix, cx)).into_any_element())
                            .collect()
                    }),
                )
                .track_scroll(self.scroll.clone())
                .flex_1(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How long searching a big project takes (NULL_BENCH_DIR), to the end:
    /// `cargo test --release -- --ignored searching_a_big_project --nocapture`.
    #[test]
    #[ignore]
    fn searching_a_big_project() {
        let Some(dir) = std::env::var_os("NULL_BENCH_DIR") else { return };
        let root = PathBuf::from(dir);
        for text in ["zzqq_never_there", "unsafe impl Send", "TODO"] {
            let query = SearchQuery { text: text.into(), ..Default::default() };
            let started = Instant::now();
            let (mut files, mut lines) = (0, 0);
            let cancel = AtomicBool::new(false);
            search_files(
                &crate::roots::ProjectRoots::new(root.clone()),
                &query,
                "",
                &[],
                None,
                &Default::default(),
                &cancel,
                |found| {
                    if let Found::Files(batch) = found {
                        files += batch.len();
                        lines += batch.iter().map(|f| f.matches.len()).sum::<usize>();
                    }
                },
            );
            println!(
                "{text:>18}: {lines:>5} lines in {files:>5} files, {:.0} ms",
                started.elapsed().as_secs_f64() * 1000.
            );
        }
    }

    /// A pattern with `\n` matches across lines: listed by the line each starts on, its end
    /// where it ends; replaced whole (only those starting on the lines asked, if any).
    #[test]
    fn patterns_across_lines() {
        let text = "fn a() {\n}\nfn b() {\r\n    x\r\n}\n";
        let query = SearchQuery { text: r"\{\r?\n\}".into(), regex: true, ..Default::default() };
        assert!(query.spans_lines());
        assert!(!SearchQuery { text: r"\s+$".into(), regex: true, ..Default::default() }.spans_lines());
        let regex = query.build().unwrap();
        let found = spanning_matches(text, &regex, 100);
        assert_eq!(found.len(), 1, "only the empty body");
        assert_eq!((found[0].line, found[0].columns.clone(), found[0].end), (0, 7..8, Some((1, 1))));
        // A Windows file's lines, and several matches.
        let body = SearchQuery { text: r"\{\r?\n[^}]*\}".into(), regex: true, ..Default::default() };
        let found = spanning_matches(text, &body.build().unwrap(), 100);
        assert_eq!(found.iter().map(|m| (m.line, m.end)).collect::<Vec<_>>(), [(0, Some((1, 1))), (2, Some((4, 1)))]);
        assert_eq!(spanning_matches(text, &body.build().unwrap(), 1).len(), 1, "at most as many as asked");
        let replaced = replacements(text, &body, "{}", None);
        assert_eq!(
            replaced.iter().map(|(r, n)| (&text[r.clone()], n.as_str())).collect::<Vec<_>>(),
            [("{\n}", "{}"), ("{\r\n    x\r\n}", "{}")]
        );
        assert_eq!(replacements(text, &body, "{}", Some(&[2])).len(), 1);
        // Starting on a Windows line's break: shown at that line's end (no crash).
        let close = SearchQuery { text: r"\n\}".into(), regex: true, ..Default::default() };
        let found = spanning_matches("a\r\n}\r\n", &close.build().unwrap(), 100);
        assert_eq!((found[0].line, found[0].columns.clone()), (0, 1..1));
        // A backslash and an n (`\\n`, as in "a\n" in a string) is a line's own text.
        assert!(!SearchQuery { text: r#""[^"]*\\n"#.into(), regex: true, ..Default::default() }.spans_lines());
        assert!(SearchQuery { text: r"\\\n".into(), regex: true, ..Default::default() }.spans_lines());
    }

    #[test]
    fn replacements_fill_in_groups_and_can_keep_to_one_line() {
        let text = "let a = old(1);\nlet b = old(2);\n";
        let plain = SearchQuery { text: "old".into(), ..Default::default() };
        assert_eq!(replacements(text, &plain, "new", None).len(), 2);
        assert_eq!(replacements(text, &plain, "new", Some(&[1])), vec![(24..27, "new".into())]);
        // In every file too, a lower-case replacement takes each match's case.
        let names: Vec<String> =
            replacements("User user USER", &SearchQuery { text: "user".into(), ..Default::default() }, "client", None)
                .into_iter()
                .map(|(_, new)| new)
                .collect();
        assert_eq!(names, ["Client", "client", "CLIENT"]);
        let (shown, _) = replace_preview("User", &[0..4], "client", None, true);
        assert_eq!(shown.as_ref(), "UserClient");
        let regex = SearchQuery { text: r"old\((\d)\)".into(), regex: true, ..Default::default() };
        let found = replacements(text, &regex, "new($1, 0)", None);
        assert_eq!(found[1].1, "new(2, 0)");
        // In a plain search, "$1" is just text.
        assert_eq!(replacements(text, &plain, "$1", Some(&[0]))[0].1, "$1");
        // A regex never takes a line break: stripping trailing spaces leaves blank lines be,
        // in a Windows file too.
        let trailing = SearchQuery { text: r"\s+$".into(), regex: true, ..Default::default() };
        let apply = |text: &str| {
            let mut out = text.to_string();
            for (range, new) in replacements(text, &trailing, "", None).into_iter().rev() {
                out.replace_range(range, &new);
            }
            out
        };
        assert_eq!(apply("foo  \n\nbar \n"), "foo\n\nbar\n");
        assert_eq!(apply("a \r\n\r\nb\r\n"), "a\r\n\r\nb\r\n");
        let commas = SearchQuery { text: r",\s*".into(), regex: true, ..Default::default() };
        assert_eq!(replacements("a,\nb", &commas, ";", None), vec![(1..2, ";".into())]);
    }

    #[test]
    fn the_files_field_picks_which_files_are_searched() {
        let root = crate::tools::test_dir("search-files");
        for file in ["src/main.rs", "src/ui/view.rs", "src/notes.md", "tests/it.rs", "docs/main.rs"] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "needle\n").unwrap();
        }
        let query = SearchQuery { text: "needle".into(), ..Default::default() };
        let found = |files: &str| {
            let mut found = Vec::new();
            search_files(
                &crate::roots::ProjectRoots::new(root.clone()),
                &query,
                files,
                &[],
                None,
                &Default::default(),
                &AtomicBool::new(false),
                |f| {
                    if let Found::Files(files) = f {
                        found.extend(files.into_iter().map(|f| f.relative.replace('\\', "/")));
                    }
                },
            );
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
        let (shown, marks) = replace_preview("a(old)", &[2..5], "new", None, false);
        assert_eq!(shown.as_ref(), "a(oldnew)");
        assert_eq!(marks, vec![(2..5, false), (5..8, true)]);
    }

    /// A match taken off the list isn't replaced: its file goes line by line; a file taken
    /// off goes whole.
    /// A search used (a result opened) is remembered first, once, with how it searched;
    /// ten at most. A recent one clicked searches again, its choices too.
    #[gpui::test]
    fn searches_used_are_remembered(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let (search, cx) = cx.add_window_view(|_, cx| ProjectSearch::new(PathBuf::from("/p"), cx));
        let m = LineMatch { line: 1, columns: 0..1, end: None, preview: "x".into(), highlights: vec![] };
        let file = FileResult { path: PathBuf::from("a"), relative: "a".into(), matches: vec![m], dismissed: false };
        let query = |text: &str, regex: bool| SearchQuery { text: text.into(), regex, ..Default::default() };
        search.update(cx, |s, cx| {
            s.set_results(vec![file], Status::Idle, cx);
            for (text, regex) in [("old", false), ("fn \\w+", true), ("old", false)] {
                s.query = query(text, regex);
                s.open(0, 0, false, cx);
            }
            assert_eq!(s.history, [query("old", false), query("fn \\w+", true)]);
            for i in 0..20 {
                s.query = query(&format!("q{i}"), false);
                s.remember();
            }
            assert_eq!(s.history.len(), 10);
            assert_eq!(s.history[0].text, "q19");
            s.history = vec![query("fn \\w+", true)];
        });
        search.update_in(cx, |s, window, cx| s.search_for(s.history[0].clone(), window, cx));
        search.read_with(cx, |s, cx| {
            assert_eq!(s.input.read(cx).text(), "fn \\w+");
            assert!(s.regex);
        });
    }

    #[gpui::test]
    fn results_taken_off_the_list(cx: &mut gpui::TestAppContext) {
        let search = cx.new(|cx| ProjectSearch::new(PathBuf::from("/p"), cx));
        let m = |line| LineMatch { line, columns: 0..1, end: None, preview: "x".into(), highlights: vec![] };
        let file = |name: &str, lines: Vec<usize>| FileResult {
            path: PathBuf::from(name),
            relative: name.into(),
            matches: lines.into_iter().map(m).collect(),
            dismissed: false,
        };
        search.update(cx, |s, cx| {
            s.set_results(vec![file("a", vec![1, 2, 5]), file("b", vec![3])], Status::Idle, cx);
            assert_eq!(replace_targets(&s.results[0]), [(PathBuf::from("a"), None)]);
            s.dismiss(0, Some(1), cx);
            assert_eq!(replace_targets(&s.results[0]), [(PathBuf::from("a"), Some(vec![1, 5]))]);
            s.dismiss(1, None, cx);
            assert_eq!(s.results.len(), 1);
            assert_eq!(s.rows.len(), 3, "a, a:1, a:5");
            // Found again (the search running after a replace): still off the list.
            assert!(s.undismissed(file("b", vec![3])).is_none());
            let again = s.undismissed(file("a", vec![1, 2, 5])).unwrap();
            assert_eq!(again.matches.iter().map(|m| m.line).collect::<Vec<_>>(), [1, 5]);
            assert!(again.dismissed);
        });
        // The keyboard on a result below a file taken off: still on a result there, and ↑
        // goes on from it.
        search.update(cx, |s, cx| {
            s.set_results(vec![file("a", vec![1, 2, 3]), file("b", vec![4, 5])], Status::Idle, cx);
            s.selected = Some(5);
            s.dismiss(0, None, cx);
            assert!(s.selected.is_some_and(|i| i < s.rows.len() && matches!(s.rows[i], Row::Match(..))));
            s.step(false, cx);
            s.step(false, cx);
        });
    }

    #[gpui::test]
    fn arrows_step_through_matches_skipping_file_names(cx: &mut gpui::TestAppContext) {
        let search = cx.new(|cx| ProjectSearch::new(PathBuf::from("/p"), cx));
        let m = |line| LineMatch { line, columns: 0..1, end: None, preview: "x".into(), highlights: vec![] };
        let file = |name: &str, lines: Vec<usize>| FileResult {
            path: PathBuf::from(name),
            relative: name.into(),
            matches: lines.into_iter().map(m).collect(),
            dismissed: false,
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

    /// Find in Folder: only that folder's files are searched.
    #[gpui::test]
    fn finds_in_one_folder(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("find-in-folder");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("lib/src")).unwrap();
        std::fs::create_dir_all(dir.join("app/[id]")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "needle\n").unwrap();
        std::fs::write(dir.join("lib/src/c.rs"), "needle\n").unwrap();
        std::fs::write(dir.join("app/[id]/page.tsx"), "needle\n").unwrap();
        std::fs::write(dir.join("b.rs"), "needle\n").unwrap();
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (search, cx) = cx.add_window_view(|_, cx| ProjectSearch::new(root, cx));
        search.update_in(cx, |s, window, cx| s.search_in_folder("src", window, cx));
        cx.simulate_input("needle");
        cx.executor().advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        let found: Vec<String> =
            search.read_with(cx, |s, _| s.results.iter().map(|r| r.relative.replace('\\', "/")).collect());
        assert_eq!(found, ["src/a.rs"], "not lib/src");
        // A folder named like a pattern is that folder.
        search.update_in(cx, |s, window, cx| s.search_in_folder("app/[id]", window, cx));
        cx.executor().advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        let found: Vec<String> =
            search.read_with(cx, |s, _| s.results.iter().map(|r| r.relative.replace('\\', "/")).collect());
        assert_eq!(found, ["app/[id]/page.tsx"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_the_notes_left_in_code() {
        let dir = crate::tools::test_dir("search-todos");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "// TODO: split\nfn a() {} // FIXME later\n// TODOS, todo, XXXL\n").unwrap();
        let mut lines = Vec::new();
        search_files(
            &crate::roots::ProjectRoots::new(dir.clone()),
            &todo_query(),
            "",
            &[],
            None,
            &Default::default(),
            &AtomicBool::new(false),
            |found| {
                if let Found::Files(files) = found {
                    lines.extend(files.into_iter().flat_map(|f| f.matches.into_iter().map(|m| m.line)));
                }
            },
        );
        lines.sort();
        assert_eq!(lines, [0, 1]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn searches_only_the_open_files_when_asked() {
        let dir = crate::tools::test_dir("search-open");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.join(name), "needle\n").unwrap();
        }
        let query = SearchQuery { text: "needle".into(), ..Default::default() };
        let found = |only: Option<&std::collections::HashSet<PathBuf>>| {
            let mut names = Vec::new();
            search_files(
                &crate::roots::ProjectRoots::new(dir.clone()),
                &query,
                "",
                &[],
                only,
                &Default::default(),
                &AtomicBool::new(false),
                |found| {
                    if let Found::Files(files) = found {
                        names.extend(
                            files.into_iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()),
                        );
                    }
                },
            );
            names
        };
        assert_eq!(found(None), ["a.txt", "b.txt", "c.txt"]);
        let open = std::collections::HashSet::from([dir.join("b.txt")]);
        assert_eq!(found(Some(&open)), ["b.txt"]);
        // An open file git ignores is searched too.
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".gitignore"), ".env\n").unwrap();
        std::fs::write(dir.join(".env"), "needle=1\n").unwrap();
        assert!(!found(None).contains(&".env".to_string()), "a project search leaves it out");
        let open = std::collections::HashSet::from([dir.join(".env"), dir.join("a.txt")]);
        assert_eq!(found(Some(&open)), [".env", "a.txt"]);
        assert!(found(Some(&Default::default())).is_empty(), "no tabs: nothing");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn searches_files_in_other_encodings() {
        let dir = crate::tools::test_dir("search-latin");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.txt"), b"caf\xe9 cr\xe8me\n").unwrap();
        let query = SearchQuery { text: "crème".into(), ..Default::default() };
        let mut results = Vec::new();
        search_files(
            &crate::roots::ProjectRoots::new(dir.clone()),
            &query,
            "",
            &[],
            None,
            &Default::default(),
            &AtomicBool::new(false),
            |found| {
                if let Found::Files(files) = found {
                    results.extend(files)
                }
            },
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches[0].preview.as_ref(), "café crème");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn line_ends_found_with_windows_line_endings() {
        let dir = crate::tools::test_dir("search-crlf");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.c"), "int a;\r\nint b\r\nint c;\r\n").unwrap();
        let query = SearchQuery { text: ";$".into(), regex: true, ..Default::default() };
        let mut lines = Vec::new();
        search_files(
            &crate::roots::ProjectRoots::new(dir.clone()),
            &query,
            "",
            &[],
            None,
            &Default::default(),
            &AtomicBool::new(false),
            |found| {
                if let Found::Files(files) = found {
                    lines.extend(files.into_iter().flat_map(|f| f.matches.into_iter().map(|m| m.line)));
                }
            },
        );
        assert_eq!(lines, [0, 2]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_matches_across_files_and_skips_ignored_ones() {
        let dir = crate::tools::test_dir("search-test");
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
        search_files(
            &crate::roots::ProjectRoots::new(dir.clone()),
            &query,
            "",
            &[],
            None,
            &Default::default(),
            &AtomicBool::new(false),
            |found| match found {
                Found::Files(files) => results.extend(files),
                Found::Done { truncated: t } => truncated = Some(t),
            },
        );
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
        search_files(
            &crate::roots::ProjectRoots::new(dir.clone()),
            &query,
            "",
            &[],
            None,
            &Default::default(),
            &AtomicBool::new(true),
            |_| reported = true,
        );
        assert!(!reported);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
