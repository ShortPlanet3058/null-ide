use crate::fonts::Fonts;
use crate::search::SearchQuery;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use crate::ui;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle, SharedString,
    StyledText, Subscription, Task, Window, div, prelude::*, px, uniform_list,
};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    Open { path: PathBuf, line: usize, columns: Range<usize>, query: SearchQuery },
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
    _subscription: Subscription,
}

impl EventEmitter<ProjectSearchEvent> for ProjectSearch {}

impl ProjectSearch {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Search in project", cx));
        let subscription = cx.subscribe(&input, |this, _, TextInputEvent::Changed, cx| this.search(cx));
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
            _subscription: subscription,
        }
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.root = root;
        self.search(cx);
    }

    /// Focuses the field, optionally replacing its text.
    pub fn focus(&mut self, text: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| match text {
            Some(text) => input.set_text(&text, cx),
            None => input.select_all_text(cx),
        });
        window.focus(&self.input.focus_handle(cx));
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let query = SearchQuery {
            text: self.input.read(cx).text().to_string(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
        };
        self.query = query.clone();
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
        // Replacing the task cancels a search that's still running.
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let (results, truncated) = cx.background_executor().spawn(async move { search_files(&root, &query) }).await;
            this.update(cx, |this, cx| {
                let matches = results.iter().map(|f| f.matches.len()).sum();
                let status = Status::Done { files: results.len(), matches, truncated };
                this.set_results(results, status, cx);
            })
            .ok();
        }));
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

    fn open(&mut self, file: usize, line_match: usize, cx: &mut Context<Self>) {
        let file = &self.results[file];
        let m = &file.matches[line_match];
        cx.emit(ProjectSearchEvent::Open {
            path: file.path.clone(),
            line: m.line,
            columns: m.columns.clone(),
            query: self.query.clone(),
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
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open(f, 0, cx)))
                    .into_any_element()
            }
            Row::Match(f, m) => {
                let line_match = &self.results[f].matches[m];
                let highlight = HighlightStyle {
                    color: Some(theme.foreground),
                    background_color: Some(theme.selection),
                    ..Default::default()
                };
                let text = StyledText::new(line_match.preview.clone())
                    .with_highlights(line_match.highlights.iter().map(|r| (r.clone(), highlight)));
                row.pl(px(10.))
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .font_family(cx.global::<Fonts>().code.clone())
                            .child(text),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open(f, m, cx)))
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
        div()
            .id(id)
            .h(px(22.))
            .min_w(px(24.))
            .px(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(ui::R_KEY))
            .font_family(cx.global::<Fonts>().code.clone())
            .text_size(px(ui::T_SM))
            .text_color(if on { theme.caret } else { theme.muted })
            .when(on, |b| b.bg(theme.accent_soft))
            .when(!on, |b| b.hover(|s| s.bg(theme.hairline).text_color(theme.foreground)))
            .child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                flip(this);
                this.search(cx);
            }))
    }
}

/// Every match of `query` in the project's files, respecting `.gitignore`.
/// Returns whether the search stopped early at [`MAX_MATCHES`].
fn search_files(root: &Path, query: &SearchQuery) -> (Vec<FileResult>, bool) {
    let Ok(regex) = query.build() else { return (Vec::new(), false) };
    let mut files: Vec<PathBuf> = ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_FILE_SIZE))
        .map(|e| e.into_path())
        .collect();
    files.sort();

    let mut results = Vec::new();
    let mut total = 0;
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
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
            results.push(FileResult { path, relative, matches });
        }
        if total >= MAX_MATCHES {
            return (results, true);
        }
    }
    (results, false)
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
                    .child(div().flex_1().min_w_0().overflow_hidden().child(self.input.clone()))
                    .child(case)
                    .child(word)
                    .child(regex),
            )
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
                .flex_1(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let (results, truncated) = search_files(&dir, &query);
        assert!(!truncated);
        assert_eq!(results.len(), 1);
        let file = &results[0];
        assert_eq!(file.relative, Path::new("src").join("a.rs").to_string_lossy());
        assert_eq!(file.matches.len(), 2);
        assert_eq!((file.matches[1].line, file.matches[1].columns.clone()), (1, 15..20));
        // The preview drops indentation and keeps the highlight on the match.
        assert_eq!(file.matches[1].preview.as_ref(), "let beta = alpha();");
        assert_eq!(file.matches[1].highlights, vec![11..16]);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
