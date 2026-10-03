use crate::ai::ProviderId;
use crate::editor::{Editor, EditorEvent, GoToDefinition, Redo, Save, SelectAll, ShowInfo, Undo};
use crate::file_tree::{FileTree, FileTreeEvent};
use crate::find_bar::{DeployFind, DeployReplace};
use crate::fonts::Fonts;
use crate::git;
use crate::key_prompt::{KeyPrompt, KeyPromptEvent};
use crate::lsp_store::{LspStore, Readiness};
use crate::menus::{self, Quit, ToggleFadeWhileTyping, ToggleWordWrap};
use crate::palette::{Category, Command, Palette, PaletteEvent, PaletteKind, PaletteOptions, format_keys};
use crate::project_search::{ProjectSearch, ProjectSearchEvent};
use crate::settings::{self, DEFAULT_FONT_SIZE, Settings};
use crate::settings_panel::{Section, SettingsPanel, SettingsPanelEvent, Shortcut};
use crate::terminal::{Shell, TerminalEvent, TerminalView};
use crate::theme::{Theme, ThemeName};
use crate::ui;
use crate::welcome::{Welcome, WelcomeEvent};
use gpui::{
    Action, AnyElement, App, ClickEvent, Context, DragMoveEvent, Entity, FocusHandle, Focusable, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels, Point, PromptLevel, SharedString,
    Subscription, Task, Window, WindowControlArea, actions, div, prelude::*, px, relative, svg,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

actions!(
    workspace,
    [
        Open,
        CloseTab,
        ToggleSidebar,
        NextTab,
        MoveTabRight,
        MoveTabLeft,
        NewAiTask,
        ShowWelcome,
        InstallShellCommand,
        ReviewAiTask,
        StopAiTask,
        KeepAllTaskChanges,
        UndoAllTaskChanges,
        PreviousTab,
        TogglePalette,
        ShowCommands,
        AskAi,
        ToggleAi,
        ShowProblems,
        ToggleFormatOnSave,
        OpenSettings,
        OpenSettingsFile,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        UseOledTheme,
        UseGraphiteTheme,
        UsePaperTheme,
        SearchProject,
        ReplaceInProject,
        ShowFiles,
        ToggleAutocomplete,
        ToggleTerminal,
        GoToLine,
        GoToSymbol,
        GoToSymbolInProject,
        NewUntitled,
        SaveAs,
        SaveAll,
        ReopenClosedTab,
        CloseAllTabs,
        CloseOtherTabs,
        UseNvidia,
        UseOllama,
        UseOpenAiCompatible,
        UseClaudeApi,
        UseClaudeCode,
        UseCodex,
        TurnOffAi,
        SetApiKey,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Workspace");
    let mut keys = vec![
        KeyBinding::new("secondary-p", TogglePalette, ctx),
        KeyBinding::new("secondary-k", ShowCommands, ctx),
        KeyBinding::new("secondary-shift-p", ShowCommands, ctx),
        KeyBinding::new("secondary-o", Open, ctx),
        KeyBinding::new("secondary-w", CloseTab, ctx),
        KeyBinding::new("secondary-b", ToggleSidebar, ctx),
        KeyBinding::new("ctrl-tab", NextTab, ctx),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, ctx),
        KeyBinding::new("secondary-,", OpenSettings, ctx),
        KeyBinding::new("secondary-shift-f", SearchProject, ctx),
        KeyBinding::new("secondary-shift-h", ReplaceInProject, ctx),
        // Arrows rather than ⌘\, which takes several keys on many layouts.
        KeyBinding::new("alt-secondary-i", NewAiTask, ctx),
        KeyBinding::new("ctrl-secondary-right", MoveTabRight, ctx),
        KeyBinding::new("ctrl-secondary-left", MoveTabLeft, ctx),
        KeyBinding::new("secondary-shift-e", ShowFiles, ctx),
        KeyBinding::new("ctrl-`", ToggleTerminal, ctx),
        KeyBinding::new("ctrl-g", GoToLine, ctx),
        KeyBinding::new("secondary-shift-m", ShowProblems, ctx),
        KeyBinding::new("secondary-shift-o", GoToSymbol, ctx),
        KeyBinding::new("secondary-t", GoToSymbolInProject, ctx),
        KeyBinding::new("alt-z", ToggleWordWrap, ctx),
        KeyBinding::new("secondary-n", NewUntitled, ctx),
        KeyBinding::new("secondary-shift-s", SaveAs, ctx),
        // Also here, not only in the editor: saving must work wherever the keyboard is
        // (the file tree after clicking a file, the terminal, the find bar...).
        KeyBinding::new("secondary-s", Save, ctx),
        KeyBinding::new("secondary-alt-s", SaveAll, ctx),
        KeyBinding::new("secondary-shift-t", ReopenClosedTab, ctx),
        KeyBinding::new("secondary-=", IncreaseFontSize, ctx),
        KeyBinding::new("secondary-+", IncreaseFontSize, ctx),
        KeyBinding::new("secondary--", DecreaseFontSize, ctx),
        KeyBinding::new("secondary-0", ResetFontSize, ctx),
    ];
    if cfg!(target_os = "macos") {
        keys.extend([KeyBinding::new("cmd-shift-]", NextTab, ctx), KeyBinding::new("cmd-shift-[", PreviousTab, ctx)]);
    }
    cx.bind_keys(keys);
}

/// Where the window is, in the session's terms.
fn window_state(window: &Window) -> crate::session::WindowState {
    let (bounds, maximized) = match window.window_bounds() {
        gpui::WindowBounds::Windowed(b) => (b, false),
        gpui::WindowBounds::Maximized(b) | gpui::WindowBounds::Fullscreen(b) => (b, true),
    };
    crate::session::WindowState {
        x: f32::from(bounds.origin.x),
        y: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
        maximized,
    }
}

/// Chrome dims to this while you type.
const DIMMED: f32 = 0.12;
const FADE_OUT: Duration = Duration::from_millis(700);
const FADE_IN: Duration = Duration::from_millis(180);
/// Mouse movement smaller than this (trackpad jitter) doesn't bring the chrome back.
const WAKE_DISTANCE: f32 = 6.;
const SIDEBAR_WIDTH: f32 = crate::file_tree::TREE_WIDTH;
/// Search results need more room than file names.
const SEARCH_SIDEBAR_WIDTH: f32 = 340.;
const SIDEBAR_SLIDE: Duration = Duration::from_millis(260);
const TERMINAL_HEIGHT: f32 = 300.;
const TERMINAL_SLIDE: Duration = Duration::from_millis(240);
/// Room for the window buttons at the left of the title bar.
const TITLEBAR_INSET: f32 = if cfg!(target_os = "macos") { 84. } else { 12. };

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
}

/// A value that eases toward 0 or 1 when switched.
struct Transition {
    on: bool,
    from: f32,
    changed_at: Instant,
}

impl Transition {
    fn new(on: bool) -> Self {
        Self { on, from: if on { 1. } else { 0. }, changed_at: Instant::now() - Duration::from_secs(1) }
    }

    /// Current value and whether it's still moving.
    fn value(&self, on_duration: Duration, off_duration: Duration) -> (f32, bool) {
        let (target, duration) = if self.on { (1., on_duration) } else { (0., off_duration) };
        let t = (Instant::now() - self.changed_at).as_secs_f32() / duration.as_secs_f32();
        (self.from + (target - self.from) * smoothstep(t), t < 1.)
    }

    fn set(&mut self, on: bool, on_duration: Duration, off_duration: Duration) {
        if on != self.on {
            self.from = self.value(on_duration, off_duration).0;
            self.on = on;
            self.changed_at = Instant::now();
        }
    }
}

/// An AI task: running in the background, then waiting for its changes to be reviewed.
struct AiTaskRun {
    title: String,
    state: TaskState,
    /// The running tool, so it can be stopped.
    child: Arc<std::sync::Mutex<Option<std::process::Child>>>,
    /// Set to stop a task running through an API (between two steps).
    stop: Arc<std::sync::atomic::AtomicBool>,
    _task: Option<Task<()>>,
}

enum TaskState {
    /// Saving open files and copying the project, to compare with afterwards.
    Starting,
    /// At work; the file it's at, when it says.
    Running(Option<String>),
    /// Done: the files it changed, still to review.
    Review(Vec<crate::ai_task::FileChange>),
}

/// A tab being dragged: to another place in the tabs, or to the other side.
#[derive(Clone)]
struct DraggedTab {
    editor: Entity<Editor>,
    label: SharedString,
}

/// What follows the pointer while a tab is dragged: its name, as a small pill.
struct TabGhost {
    label: SharedString,
}

impl Render for TabGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        div()
            .px(px(12.))
            .py(px(4.))
            .rounded(px(ui::R_CONTROL))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.line_strong)
            .shadow_md()
            .text_size(px(ui::T_MD))
            .text_color(theme.foreground)
            .font_family(cx.global::<Fonts>().ui.clone())
            .child(self.label.clone())
    }
}

/// The line between the two sides, being dragged to resize them.
#[derive(Clone)]
struct DraggedDivider;

struct Tab {
    editor: Entity<Editor>,
    /// Which side it's on: 0 left (or the only one), 1 right.
    side: usize,
    _subscriptions: [Subscription; 2],
}

pub struct Workspace {
    focus_handle: FocusHandle,
    tree: Entity<FileTree>,
    lsp: Entity<LspStore>,
    project_search: Entity<ProjectSearch>,
    /// On while the sidebar shows project search instead of files.
    sidebar_search: Transition,
    tabs: Vec<Tab>,
    /// The tab with the keyboard: the one shown on the side being worked in.
    active: Option<usize>,
    /// The tab each side shows.
    shown: [Option<Entity<Editor>>; 2],
    /// How much of the width the left side takes when split.
    split_ratio: f32,
    sidebar: Transition,
    chrome: Transition,
    last_mouse: Option<Point<Pixels>>,
    palette: Option<(Entity<Palette>, Subscription)>,
    settings_panel: Option<(Entity<SettingsPanel>, Subscription)>,
    /// The first-launch screen, until it's been seen.
    welcome: Option<(Entity<Welcome>, Subscription)>,
    /// Files whose tabs were closed, most recent last, for Cmd+Shift+T.
    recently_closed: Vec<PathBuf>,
    /// Files activated lately, most recent first, for the palette.
    recent_files: Vec<PathBuf>,
    /// Actions run from the palette lately, most recent first.
    recent_commands: Vec<&'static str>,
    /// Watches the project folder so the tree and open files follow changes made elsewhere.
    _watcher: Option<notify::RecommendedWatcher>,
    watch_task: Option<Task<()>>,
    index_task: Option<Task<()>>,
    session_task: Option<Task<()>>,
    /// The tab strip, scrolled so the current tab is always in view.
    tab_scroll: [gpui::ScrollHandle; 2],
    /// The window's place on screen, for the session.
    window_state: Option<crate::session::WindowState>,
    reindex_task: Option<Task<()>>,
    /// The editor and its view (caret line, column, top line) before a symbol list
    /// started showing places in it.
    view_before_preview: Option<(Entity<Editor>, (usize, usize, usize))>,
    /// The AI task running or waiting for review, if any.
    ai_task: Option<AiTaskRun>,
    /// Commands the next palette list shows after its places (a task's "Keep all").
    pending_commands: Vec<crate::palette::Command>,
    /// Changed source files waiting to be read again.
    reindex_pending: HashSet<PathBuf>,
    /// What the project's .gitignore leaves out of the index.
    ignore_rules: ignore::gitignore::Gitignore,
    /// The AI answer panel, while open.
    key_prompt: Option<(Entity<KeyPrompt>, Subscription)>,
    /// A short message at the bottom of the window, and when it appeared.
    notice: Option<(String, Instant)>,
    notice_task: Option<Task<()>>,
    /// The terminal, once opened. It keeps running while the panel is hidden.
    terminal: Option<(Entity<TerminalView>, Subscription)>,
    terminal_open: Transition,
    /// The current git branch of the project, if it's a repository.
    branch: Option<String>,
    branch_task: Option<Task<()>>,
    /// When language support first became ready this session, to show a short tip once.
    ready_since: Option<Instant>,
    /// Where focus goes back to when the palette closes.
    focus_before_palette: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project_search = cx.new(|cx| ProjectSearch::new(root.clone(), cx));
        let lsp = cx.new(|_| LspStore::new(root.clone()));
        let ignore_rules = crate::project_index::ignore_rules(&root);
        let tree = cx.new(|cx| FileTree::new(root, cx));
        let subscriptions = vec![
            cx.subscribe_in(&tree, window, |this, _, event, window, cx| match event {
                FileTreeEvent::Open(path) | FileTreeEvent::Created(path) => this.open_file(path.clone(), window, cx),
                FileTreeEvent::Preview(path) => {
                    this.open_file(path.clone(), window, cx);
                    window.focus(&this.tree.focus_handle(cx));
                }
                FileTreeEvent::Renamed { from, to } => this.paths_renamed(from, to, cx),
                FileTreeEvent::Trashed(path) => this.path_trashed(path, window, cx),
                FileTreeEvent::Notice(message) => this.show_notice(message.clone(), cx),
            }),
            cx.observe_global::<Settings>(|this, cx| this.apply_settings(cx)),
            cx.observe(&lsp, |_, _, cx| cx.notify()),
            cx.subscribe(&lsp, |this, _, event, cx| {
                if let crate::lsp_store::LspEvent::Installed(result) = event {
                    let message = match result {
                        Ok(name) => format!("{name} is installed"),
                        Err(reason) => reason.clone(),
                    };
                    this.show_notice(message, cx);
                }
            }),
            cx.subscribe_in(&project_search, window, |this, _, event, window, cx| match event {
                ProjectSearchEvent::Open { path, line, columns, query, keep_focus } => {
                    let (line, columns, query) = (*line, columns.clone(), query.clone());
                    this.open_file(path.clone(), window, cx);
                    if let Some(editor) = this.active_editor() {
                        editor.update(cx, |editor, cx| editor.reveal_match(line, columns, query, cx));
                    }
                    if *keep_focus {
                        let search = this.project_search.focus_handle(cx);
                        window.focus(&search);
                    }
                }
                ProjectSearchEvent::Replace { query, replacement, targets } => {
                    let (query, replacement, targets) = (query.clone(), replacement.clone(), targets.clone());
                    this.replace_in_files(&query, &replacement, &targets, cx);
                }
            }),
        ];
        let this = cx.entity().downgrade();
        // Coming back to the window: files may have been committed or the branch switched meanwhile.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh_git(cx);
            }
        })
        .detach();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.confirm_unsaved(CloseAction::CloseWindow, window, cx)).unwrap_or(true)
        });
        let mut workspace = Self {
            focus_handle: cx.focus_handle(),
            tree,
            lsp,
            project_search,
            sidebar_search: Transition::new(false),
            tabs: Vec::new(),
            shown: [None, None],
            split_ratio: 0.5,
            active: None,
            sidebar: Transition::new(cx.global::<Settings>().sidebar_visible),
            chrome: Transition::new(true),
            last_mouse: None,
            palette: None,
            settings_panel: None,
            welcome: None,
            ready_since: None,
            branch: None,
            branch_task: None,
            terminal: None,
            terminal_open: Transition::new(false),
            recently_closed: Vec::new(),
            recent_files: Vec::new(),
            recent_commands: Vec::new(),
            _watcher: None,
            watch_task: None,
            index_task: None,
            session_task: None,
            tab_scroll: [gpui::ScrollHandle::new(), gpui::ScrollHandle::new()],
            window_state: None,
            reindex_task: None,
            reindex_pending: HashSet::new(),
            view_before_preview: None,
            ai_task: None,
            pending_commands: Vec::new(),
            ignore_rules,
            key_prompt: None,
            notice: None,
            notice_task: None,
            focus_before_palette: None,
            _subscriptions: subscriptions,
        };
        workspace.refresh_git(cx);
        workspace.watch(workspace.tree.read(cx).root().to_path_buf(), cx);
        workspace.build_index(cx);
        // Save the session when quitting, when the window moves or resizes, and when Null
        // goes to the background (so a crash loses little).
        cx.on_app_quit(|this, cx| {
            this.save_session(cx);
            async {}
        })
        .detach();
        workspace._subscriptions.push(cx.observe_window_bounds(window, |this, window, cx| {
            this.window_state = Some(window_state(window));
            this.schedule_session_save(cx);
        }));
        workspace._subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.save_session(cx);
            }
        }));
        workspace
    }

    /// The project as it is now: tabs with their caret and scroll, open folders, terminal, window.
    fn session(&self, cx: &App) -> crate::session::Session {
        let tabs = self
            .tabs
            .iter()
            .filter_map(|tab| {
                let editor = tab.editor.read(cx);
                let (line, column, top_line) = editor.view_state();
                Some(crate::session::TabState {
                    path: editor.path()?.to_path_buf(),
                    line,
                    column,
                    top_line,
                    side: tab.side,
                    folds: editor.folded_regions(),
                })
            })
            .collect::<Vec<_>>();
        // The active tab among those saved (untitled tabs aren't).
        let active = self.active_editor().and_then(|e| e.read(cx).path().map(Path::to_path_buf));
        let right = self.shown_editor(1).and_then(|e| e.read(cx).path().map(Path::to_path_buf));
        crate::session::Session {
            active: active.and_then(|p| tabs.iter().position(|t| t.path == p)),
            shown_right: right.and_then(|p| tabs.iter().position(|t| t.path == p)),
            split_ratio: self.is_split().then_some(self.split_ratio),
            tabs,
            expanded: self.tree.read(cx).expanded_folders(),
            recent_files: self.recent_files.iter().take(20).cloned().collect(),
            terminal_open: self.terminal_open.on,
            window: self.window_state,
        }
    }

    fn save_session(&self, cx: &App) {
        self.session(cx).save(self.tree.read(cx).root());
    }

    /// Saves a moment after things settle, rather than on every change.
    fn schedule_session_save(&mut self, cx: &mut Context<Self>) {
        self.session_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(800)).await;
            this.update(cx, |this, cx| this.save_session(cx)).ok();
        }));
    }

    /// Opens the project as it was: its tabs (caret and scroll included), folders and terminal.
    pub fn restore_session(&mut self, session: crate::session::Session, window: &mut Window, cx: &mut Context<Self>) {
        self.tree.update(cx, |tree, cx| tree.expand_folders(&session.expanded, cx));
        self.window_state = session.window;
        for tab in &session.tabs {
            self.open_file_on(tab.path.clone(), Some(tab.side.min(1)), window, cx);
            if let Some(editor) = self.active_editor() {
                editor.update(cx, |editor, cx| {
                    editor.restore_folds(&tab.folds, cx);
                    editor.restore_view(tab.line, tab.column, tab.top_line, cx)
                });
            }
        }
        if let Some(ratio) = session.split_ratio {
            self.split_ratio = ratio.clamp(0.2, 0.8);
        }
        // What the right side showed, then the tab that had the keyboard.
        if let Some(right) = session.shown_right.and_then(|i| session.tabs.get(i)) {
            self.open_file(right.path.clone(), window, cx);
        }
        if let Some(active) = session.active.and_then(|i| session.tabs.get(i)) {
            self.open_file(active.path.clone(), window, cx);
        }
        // Opening the tabs moved each to the front of the recent files: put the saved order back.
        for path in session.recent_files.iter().rev() {
            self.recent_files.retain(|p| p != path);
            self.recent_files.insert(0, path.clone());
        }
        if session.terminal_open && !self.terminal_open.on {
            self.toggle_terminal(&ToggleTerminal, window, cx);
        }
        if self.tabs.is_empty() {
            window.focus(&self.focus_handle);
        }
    }

    /// Finds what the project defines, for suggestions, without slowing anything down.
    fn build_index(&mut self, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        cx.set_global(crate::project_index::ProjectContext { root: root.clone(), ..Default::default() });
        self.index_task = Some(cx.spawn(async move |this, cx| {
            let definitions = cx.background_executor().spawn(async move { crate::project_index::index(&root) }).await;
            this.update(cx, |this, cx| {
                cx.global_mut::<crate::project_index::ProjectContext>().definitions = std::sync::Arc::new(definitions);
                // Done: "still reading" no longer applies.
                this.index_task = None;
            })
            .ok();
        }));
    }

    /// Re-reads the definitions of source files that changed, leaving out ignored ones
    /// (a build writing into `target/`, an install filling `node_modules`). Changes that
    /// arrive while a batch is being read wait for it, so none are lost.
    fn reindex(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if !cx.has_global::<crate::project_index::ProjectContext>() {
            return;
        }
        let root = self.tree.read(cx).root().to_path_buf();
        if paths.iter().any(|p| p.file_name().is_some_and(|n| n == ".gitignore" || n == "exclude")) {
            self.ignore_rules = crate::project_index::ignore_rules(&root);
        }
        let rules = &self.ignore_rules;
        self.reindex_pending.extend(
            paths
                .iter()
                .filter(|p| crate::project_index::is_source(p) && !crate::project_index::is_ignored(rules, &root, p))
                .cloned(),
        );
        if self.reindex_pending.is_empty() || self.reindex_task.is_some() {
            return;
        }
        self.reindex_task = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok(Some((changed, current))) = this.update(cx, |this, cx| {
                    if this.reindex_pending.is_empty() {
                        this.reindex_task = None;
                        return None;
                    }
                    let changed = std::mem::take(&mut this.reindex_pending);
                    Some((changed, cx.global::<crate::project_index::ProjectContext>().definitions.clone()))
                }) else {
                    return;
                };
                let updated = cx
                    .background_executor()
                    .spawn(async move {
                        let mut definitions: Vec<_> =
                            current.iter().filter(|d| !changed.contains(&d.path)).cloned().collect();
                        for path in &changed {
                            if let Ok(text) = std::fs::read_to_string(path) {
                                definitions.extend(crate::project_index::definitions_in(path, &text));
                            }
                        }
                        definitions
                    })
                    .await;
                cx.update(|cx| {
                    cx.global_mut::<crate::project_index::ProjectContext>().definitions = std::sync::Arc::new(updated)
                })
                .ok();
            }
        }));
    }

    /// Tells suggestions which other files are open, most recently used first.
    fn share_open_files(&mut self, cx: &mut Context<Self>) {
        const OPEN_FILES: usize = 3;
        const OPEN_FILE_CHARS: usize = 3_000;
        let active = self.active_editor().and_then(|e| e.read(cx).path().map(Path::to_path_buf));
        let open: Vec<(PathBuf, String)> = self
            .recent_files
            .iter()
            .filter(|p| Some(*p) != active.as_ref())
            .filter_map(|p| {
                let tab = self.tabs.iter().find(|t| t.editor.read(cx).path() == Some(p.as_path()))?;
                let text: String = tab.editor.read(cx).buffer.to_string().chars().take(OPEN_FILE_CHARS).collect();
                Some((p.clone(), text))
            })
            .take(OPEN_FILES)
            .collect();
        if cx.has_global::<crate::project_index::ProjectContext>() {
            cx.global_mut::<crate::project_index::ProjectContext>().open_files = open;
        }
    }

    /// Watches the project folder; changes made elsewhere (terminal, git, other apps)
    /// show up in the tree and in open files.
    fn watch(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        use notify::Watcher;
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Vec<PathBuf>>();
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            if let Ok(event) = result
                && !matches!(event.kind, notify::EventKind::Access(_))
            {
                tx.unbounded_send(event.paths).ok();
            }
        });
        let Ok(mut watcher) = watcher else { return };
        if watcher.watch(&root, notify::RecursiveMode::Recursive).is_err() {
            return;
        }
        self._watcher = Some(watcher);
        self.watch_task = Some(cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            while let Some(first) = rx.next().await {
                // Changes come in bursts (a build, a checkout): wait for it to settle.
                cx.background_executor().timer(Duration::from_millis(150)).await;
                let mut paths = first;
                while let Ok(more) = rx.try_recv() {
                    paths.extend(more);
                }
                paths.sort();
                paths.dedup();
                if this.update(cx, |this, cx| this.files_changed(paths, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn files_changed(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let git_changed = paths.iter().any(|p| git::is_state_change(p));
        let visible: Vec<PathBuf> =
            paths.into_iter().filter(|p| !p.components().any(|c| c.as_os_str() == ".git")).collect();
        self.tree.update(cx, |tree, cx| tree.refresh(&visible, cx));
        self.reindex(&visible, cx);
        for tab in &self.tabs {
            let changed = tab.editor.read(cx).path().is_some_and(|p| visible.iter().any(|v| v == p));
            if changed {
                tab.editor.update(cx, |editor, cx| editor.reload_from_disk(cx));
            }
        }
        if git_changed {
            self.refresh_git(cx);
        }
    }

    /// Re-reads the branch and every open file's committed version.
    fn refresh_git(&mut self, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        self.branch_task = Some(cx.spawn(async move |this, cx| {
            let branch = cx.background_executor().spawn(async move { git::current_branch(&root) }).await;
            this.update(cx, |this, cx| {
                this.branch = branch;
                cx.notify();
            })
            .ok();
        }));
        for tab in &self.tabs {
            tab.editor.update(cx, |editor, cx| editor.reload_git_base(cx));
        }
    }

    /// Points open tabs at their new location after a file or folder was renamed.
    fn paths_renamed(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            let moved = tab.editor.read(cx).path().and_then(|p| moved_path(p, from, to));
            if let Some(new_path) = moved {
                let lsp = self.lsp.clone();
                tab.editor.update(cx, |editor, cx| editor.set_path(new_path, Some(lsp), cx));
            }
        }
        cx.notify();
    }

    /// Closes tabs whose file went to the Trash, unless they have unsaved edits.
    fn path_trashed(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let gone: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                let editor = t.editor.read(cx);
                editor.path().is_some_and(|p| p.starts_with(path)) && !editor.buffer.is_dirty()
            })
            .map(|(i, _)| i)
            .collect();
        for ix in gone.into_iter().rev() {
            self.remove_tab(ix, window, cx);
        }
    }

    fn active_editor(&self) -> Option<&Entity<Editor>> {
        self.active.and_then(|ix| self.tabs.get(ix)).map(|tab| &tab.editor)
    }

    /// Opens a file in a tab, or switches to it if it's already open (on either side).
    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file_on(path, None, window, cx);
    }

    /// Opens a file on `side`, or the side being worked in.
    fn open_file_on(&mut self, path: PathBuf, side: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|tab| tab.editor.read(cx).path() == Some(path.as_path())) {
            self.activate(ix, window, cx);
            return;
        }
        let lsp = self.lsp.clone();
        let editor = cx.new(|cx| Editor::open(path, Some(lsp), cx));
        self.add_tab_on(editor, side, window, cx);
    }

    fn is_split(&self) -> bool {
        self.tabs.iter().any(|t| t.side == 1)
    }

    /// The side being worked in.
    fn focused_side(&self) -> usize {
        self.active.and_then(|i| self.tabs.get(i)).map_or(0, |t| t.side)
    }

    /// The editor a side shows, if it shows one.
    fn shown_editor(&self, side: usize) -> Option<&Entity<Editor>> {
        self.shown[side].as_ref().filter(|e| self.tabs.iter().any(|t| &t.editor == *e && t.side == side))
    }

    /// Tabs on `side`, by index.
    fn side_tabs(&self, side: usize) -> Vec<usize> {
        (0..self.tabs.len()).filter(|&i| self.tabs[i].side == side).collect()
    }

    fn add_tab(&mut self, editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab_on(editor, None, window, cx);
    }

    fn add_tab_on(&mut self, editor: Entity<Editor>, side: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let side = side.unwrap_or_else(|| self.focused_side());
        let subscriptions = [
            cx.observe(&editor, |_, _, cx| cx.notify()),
            cx.subscribe_in(&editor, window, |this, editor, event, window, cx| match event {
                EditorEvent::Edited => {
                    this.share_unsaved(&editor, cx);
                    if cx.global::<Settings>().fade_bars_while_typing {
                        this.chrome.set(false, FADE_IN, FADE_OUT);
                    }
                    this.refresh_title(window, cx);
                    cx.notify();
                }
                // Hand edits to the settings file take effect when saved.
                EditorEvent::Saved => {
                    this.share_unsaved(&editor, cx);
                    this.refresh_title(window, cx);
                    if editor.read(cx).path().is_some_and(|p| Some(p) == Settings::path().as_deref()) {
                        settings::reload(cx);
                    }
                    cx.notify();
                }
                EditorEvent::NeedsPath => this.ask_where_to_save(editor.clone(), window, cx),
                EditorEvent::Reviewed => this.file_reviewed(&editor, cx),
                EditorEvent::SaveFailed(message) => this.show_notice(message.clone(), cx),
                EditorEvent::ChangedOnDisk => {
                    let name = editor.read(cx).file_name();
                    this.show_notice(format!("{name} changed on disk. Your unsaved edits were kept."), cx);
                }
                EditorEvent::Rename { position, new_name } => {
                    let Some(path) = editor.read(cx).path().map(Path::to_path_buf) else { return };
                    let request = this.lsp.read(cx).rename(&path, *position, new_name.clone());
                    cx.spawn_in(window, async move |this, cx| {
                        let result = request.await;
                        this.update_in(cx, |this, _, cx| match result {
                            Ok(edit) => this.apply_workspace_edit(edit, cx),
                            Err(message) => this.show_notice(message, cx),
                        })
                        .ok();
                    })
                    .detach();
                }
                EditorEvent::FindReferences { position, name } => {
                    let Some(path) = editor.read(cx).path().map(Path::to_path_buf) else { return };
                    let request = this.lsp.read(cx).references(&path, *position);
                    let name = name.clone();
                    cx.spawn_in(window, async move |this, cx| {
                        let found = request.await;
                        this.update_in(cx, |this, window, cx| {
                            let locations: Vec<_> = found
                                .into_iter()
                                .filter_map(|l| {
                                    let path = crate::lsp::path_for(&l.uri)?;
                                    let text = this.line_text(&path, l.range.start.line as usize, cx);
                                    Some(crate::palette::Location {
                                        path,
                                        position: l.range.start,
                                        text: text.trim().to_string(),
                                        kind: crate::palette::LocationKind::Reference,
                                    })
                                })
                                .collect();
                            let mut locations = locations;
                            locations.sort_by(|a, b| {
                                (&a.path, a.position.line, a.position.character).cmp(&(
                                    &b.path,
                                    b.position.line,
                                    b.position.character,
                                ))
                            });
                            if locations.len() <= 1 {
                                return this.show_notice(format!("{name} isn't used anywhere else"), cx);
                            }
                            let title = format!("{} uses of {name}", locations.len());
                            this.open_locations(title, locations, window, cx);
                        })
                        .ok();
                    })
                    .detach();
                }
                EditorEvent::GoTo { path, range } => {
                    let range = *range;
                    this.open_file(path.clone(), window, cx);
                    if let Some(editor) = this.active_editor() {
                        editor.update(cx, |editor, cx| editor.select_lsp_range(range, cx));
                    }
                }
            }),
        ];
        // Next to the current tab when it's on the same side, else at the end of that side.
        let ix = match self.active {
            Some(a) if self.tabs.get(a).is_some_and(|t| t.side == side) => a + 1,
            _ => self.side_tabs(side).last().map_or(self.tabs.len(), |&i| i + 1),
        };
        if let Some(a) = self.active.filter(|&a| a >= ix) {
            self.active = Some(a + 1);
        }
        self.tabs.insert(ix, Tab { editor, side, _subscriptions: subscriptions });
        self.activate(ix, window, cx);
    }

    /// Opens a folder as the project (its tabs replace these, after asking about unsaved
    /// changes), or a file in a tab.
    /// Opens what the Finder or the `null` command handed over: files in tabs, a folder
    /// as the project (unless it's this one already).
    pub fn open_paths(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        for path in paths {
            if path == root {
                continue;
            }
            self.open_path(path, window, cx);
        }
    }

    /// Installs `null`, to open files and folders from a terminal: `null .`, `null a.rs`.
    /// Into /usr/local/bin when it can write there, else ~/.local/bin.
    fn install_shell_command(&mut self, _: &InstallShellCommand, _: &mut Window, cx: &mut Context<Self>) {
        let Ok(exe) = std::env::current_exe() else { return };
        // From the app, `open -a` hands paths to the running Null; from a build, run it directly.
        let bundle = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf);
        let launch = match &bundle {
            Some(app) => format!("open -a \"{}\" \"$p\"", app.display()),
            None => format!("\"{}\" \"$p\" &", exe.display()),
        };
        let script = shell_script(&launch);
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let system = PathBuf::from("/usr/local/bin");
        let writable = |dir: &Path| {
            let probe = dir.join(".null-write-test");
            let ok = std::fs::write(&probe, "").is_ok();
            std::fs::remove_file(&probe).ok();
            ok
        };
        let dir = if writable(&system) { system } else { home.join(".local/bin") };
        let target = dir.join("null");
        let result = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&target, script)).and_then(|()| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
            }
            Ok(())
        });
        let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == dir));
        let message = match result {
            Ok(()) if on_path || dir.starts_with("/usr/local") => {
                format!("Installed {}: try `null .` in a terminal.", target.display())
            }
            Ok(()) => {
                format!(
                    "Installed {}. Add {} to your PATH to use `null` from a terminal.",
                    target.display(),
                    dir.display()
                )
            }
            Err(error) => format!("Couldn't install the command: {error}"),
        };
        self.show_notice(message, cx);
    }

    fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if path.is_dir() {
            // The project being left keeps its session.
            self.save_session(cx);
            self.confirm_unsaved(CloseAction::SwitchProject(path), window, cx);
        } else {
            self.open_file(path, window, cx);
        }
    }

    fn switch_project(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.project_search.update(cx, |search, cx| search.set_root(path.clone(), cx));
        // Language servers work per project: restart them for the new folder.
        let root = path.clone();
        self.lsp.update(cx, |lsp, _| {
            lsp.shutdown();
            *lsp = LspStore::new(root);
        });
        self.recent_files.clear();
        self.recently_closed.clear();
        self.refresh_git(cx);
        self.watch(path.clone(), cx);
        self.tree.update(cx, |tree, cx| tree.set_root(path.clone(), cx));
        self.ignore_rules = crate::project_index::ignore_rules(&path);
        self.reindex_pending.clear();
        self.build_index(cx);
        let mut session = crate::session::Session::load(&path);
        session.window = self.window_state;
        self.restore_session(session, window, cx);
        self.schedule_session_save(cx);
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.show_tab(ix, true, window, cx);
    }

    /// Makes tab `ix` the current one; `focus` also moves the keyboard to it.
    fn show_tab(&mut self, ix: usize, focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        self.active = Some(ix);
        self.shown[tab.side] = Some(tab.editor.clone());
        let side = tab.side;
        let position = self.side_tabs(side).iter().position(|&i| i == ix).unwrap_or(0);
        let tab = &self.tabs[ix];
        let editor = tab.editor.read(cx);
        let path = editor.path().map(Path::to_path_buf);
        if let Some(path) = &path {
            self.recent_files.retain(|p| p != path);
            self.recent_files.insert(0, path.clone());
            self.recent_files.truncate(50);
        }
        if focus {
            window.focus(&editor.focus_handle(cx));
        }
        self.refresh_title(window, cx);
        self.tab_scroll[side].scroll_to_item(position);
        self.share_open_files(cx);
        self.schedule_session_save(cx);
        self.tree.update(cx, |tree, cx| tree.set_active(path, cx));
        cx.notify();
    }

    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.schedule_session_save(cx);
        let tab = self.tabs.remove(ix);
        if let Some(path) = tab.editor.read(cx).path().map(Path::to_path_buf)
            && cx.has_global::<crate::project_search::UnsavedFiles>()
        {
            cx.global_mut::<crate::project_search::UnsavedFiles>().0.remove(&path);
        }
        // Focus follows to the next tab only if it was in the tab being closed (deleting a
        // file from the tree keeps the keyboard in the tree).
        let had_focus = tab.editor.focus_handle(cx).contains_focused(window, cx);
        if let Some(path) = tab.editor.read(cx).path() {
            self.recently_closed.push(path.to_path_buf());
        }
        tab.editor.update(cx, |editor, cx| editor.release_lsp(cx));
        let was_active = self.active == Some(ix);
        if let Some(a) = self.active.filter(|&a| a > ix) {
            self.active = Some(a - 1);
        }
        // The side it was on shows its neighbour instead.
        let same_side = self.side_tabs(tab.side);
        let neighbour = same_side.iter().find(|&&i| i >= ix).or(same_side.last()).copied();
        if self.shown[tab.side].as_ref() == Some(&tab.editor) {
            self.shown[tab.side] = neighbour.map(|i| self.tabs[i].editor.clone());
        }
        // Nothing left on the left: the right side becomes the only one.
        if self.side_tabs(0).is_empty() && self.is_split() {
            for t in &mut self.tabs {
                t.side = 0;
            }
            self.shown = [self.shown[1].take(), None];
        }
        if self.tabs.is_empty() {
            self.active = None;
            self.shown = [None, None];
            self.refresh_title(window, cx);
            if had_focus {
                window.focus(&self.focus_handle);
            }
            self.tree.update(cx, |tree, cx| tree.set_active(None, cx));
            cx.notify();
        } else if was_active {
            let other =
                self.shown_editor(1 - tab.side.min(1)).and_then(|e| self.tabs.iter().position(|t| &t.editor == e));
            let next = neighbour.or(other).unwrap_or(0);
            self.show_tab(next.min(self.tabs.len() - 1), had_focus, window, cx);
        } else {
            self.refresh_title(window, cx);
            cx.notify();
        }
    }

    /// Moves the current tab to the other side (⌃⌘→ / ⌃⌘←), opening the split when needed.
    fn move_tab_to(&mut self, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else { return };
        let from = self.tabs[ix].side;
        if from == to {
            return;
        }
        if self.tabs.len() < 2 {
            return self.show_notice("Open another file to see two side by side".into(), cx);
        }
        self.tabs[ix].side = to;
        let left_behind = self.side_tabs(from);
        let neighbour = left_behind.iter().find(|&&i| i >= ix).or(left_behind.last());
        self.shown[from] = neighbour.map(|&i| self.tabs[i].editor.clone());
        if self.side_tabs(0).is_empty() {
            for t in &mut self.tabs {
                t.side = 0;
            }
            self.shown = [None, None];
        }
        self.show_tab(ix, true, window, cx);
    }

    /// A tab dropped on `side`: before tab `before`, or at the end of that side.
    fn drop_tab(
        &mut self,
        editor: &Entity<Editor>,
        before: Option<usize>,
        side: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(from) = self.tabs.iter().position(|t| &t.editor == editor) else { return };
        let old_side = self.tabs[from].side;
        if side == 1 && old_side == 0 && self.side_tabs(0).len() == 1 && !self.is_split() {
            return self.show_notice("Open another file to see two side by side".into(), cx);
        }
        let active = self.active.map(|i| self.tabs[i].editor.clone());
        let target = before.map(|i| self.tabs[i].editor.clone());
        let mut tab = self.tabs.remove(from);
        tab.side = side;
        let at = match target.and_then(|t| self.tabs.iter().position(|x| x.editor == t)) {
            Some(i) => i,
            None => self.side_tabs(side).last().map_or(self.tabs.len(), |&i| i + 1),
        };
        self.tabs.insert(at, tab);
        self.active = active.and_then(|a| self.tabs.iter().position(|t| t.editor == a));
        // The side it left shows another of its tabs.
        if old_side != side && self.shown[old_side].as_ref() == Some(editor) {
            let left = self.side_tabs(old_side);
            self.shown[old_side] = left.first().map(|&i| self.tabs[i].editor.clone());
        }
        if self.side_tabs(0).is_empty() {
            for t in &mut self.tabs {
                t.side = 0;
            }
            self.shown = [None, None];
        }
        self.show_tab(at, true, window, cx);
    }

    /// Clicking into a side makes its tab the current one.
    fn focus_side(&mut self, side: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.shown_editor(side).and_then(|e| self.tabs.iter().position(|t| &t.editor == e)) else {
            return;
        };
        if self.active != Some(ix) {
            self.show_tab(ix, false, window, cx);
        }
    }

    /// "file — project" in the title bar, and the close button's unsaved dot.
    fn refresh_title(&self, window: &mut Window, cx: &App) {
        let project = self.tree.read(cx).root().file_name().map(|n| n.to_string_lossy().into_owned());
        let editor = self.active_editor().map(|e| e.read(cx));
        let title = match (editor.map(|e| e.file_name()), project) {
            (Some(file), Some(project)) => format!("{file} — {project}"),
            (Some(file), None) => file,
            (None, Some(project)) => project,
            (None, None) => "Null".into(),
        };
        window.set_window_title(&title);
        window.set_window_edited(self.tabs.iter().any(|t| t.editor.read(cx).buffer.is_dirty()));
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        // The index can be out of date (a click on a tab already gone).
        let Some(editor) = self.tabs.get(ix).map(|t| t.editor.clone()) else { return };
        self.confirm_unsaved(CloseAction::CloseTabs(vec![editor]), window, cx);
    }

    /// Asks before discarding unsaved changes. Returns true when `action` can go ahead
    /// right away; otherwise it runs after the person answers.
    fn confirm_unsaved(&mut self, action: CloseAction, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let scope: Vec<Entity<Editor>> = match &action {
            CloseAction::CloseTabs(editors) => editors.clone(),
            _ => self.tabs.iter().map(|tab| tab.editor.clone()).collect(),
        };
        let dirty: Vec<Entity<Editor>> = scope.iter().filter(|e| e.read(cx).buffer.is_dirty()).cloned().collect();
        if dirty.is_empty() {
            self.finish_close(action, window, cx);
            return true;
        }
        let message = match dirty.as_slice() {
            [one] => format!("Save changes to {}?", one.read(cx).file_name()),
            many => format!("Save changes to {} files?", many.len()),
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some("Your changes will be lost if you don't save them."),
            &["Save", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(choice) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                if choice == 2 {
                    return;
                }
                if choice == 1 {
                    return this.finish_close(action, window, cx);
                }
                // Save: files with a name right away, then ask where to put each untitled one.
                let (named, untitled): (Vec<_>, Vec<_>) = dirty.into_iter().partition(|e| e.read(cx).path().is_some());
                if named.iter().all(|editor| editor.update(cx, |editor, cx| editor.save_to_disk(cx))) {
                    this.save_untitled_then(untitled, action, window, cx);
                }
            })
            .ok();
        })
        .detach();
        false
    }

    /// Asks where to save each untitled file in turn, then goes on with `action`.
    /// Cancelling any of them stops there, so nothing unsaved is lost.
    fn save_untitled_then(
        &mut self,
        mut pending: Vec<Entity<Editor>>,
        action: CloseAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = pending.first().cloned() else { return self.finish_close(action, window, cx) };
        // Show which file the dialog is about.
        if let Some(ix) = self.tabs.iter().position(|t| t.editor == editor) {
            self.activate(ix, window, cx);
        }
        let root = self.tree.read(cx).root().to_path_buf();
        let answer = cx.prompt_for_new_path(&root, None);
        let lsp = self.lsp.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                let saved = editor.update(cx, |editor, cx| {
                    editor.set_path(path, Some(lsp), cx);
                    editor.save_to_disk(cx)
                });
                if saved {
                    pending.remove(0);
                    this.save_untitled_then(pending, action, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn finish_close(&mut self, action: CloseAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            CloseAction::Quit => cx.quit(),
            CloseAction::SwitchProject(path) => {
                while !self.tabs.is_empty() {
                    self.remove_tab(self.tabs.len() - 1, window, cx);
                }
                self.switch_project(path, window, cx);
            }
            CloseAction::CloseWindow => window.remove_window(),
            CloseAction::CloseTabs(editors) => {
                for editor in editors {
                    if let Some(ix) = self.tabs.iter().position(|t| t.editor == editor) {
                        self.remove_tab(ix, window, cx);
                    }
                }
            }
        }
    }

    fn close_all_tabs(&mut self, _: &CloseAllTabs, window: &mut Window, cx: &mut Context<Self>) {
        let editors = self.tabs.iter().map(|t| t.editor.clone()).collect();
        self.confirm_unsaved(CloseAction::CloseTabs(editors), window, cx);
    }

    fn close_other_tabs(&mut self, _: &CloseOtherTabs, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.active_editor().cloned();
        let editors = self.tabs.iter().map(|t| t.editor.clone()).filter(|e| Some(e) != active.as_ref()).collect();
        self.confirm_unsaved(CloseAction::CloseTabs(editors), window, cx);
    }

    fn reopen_closed_tab(&mut self, _: &ReopenClosedTab, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(path) = self.recently_closed.pop() {
            if path.exists() {
                return self.open_file(path, window, cx);
            }
        }
    }

    /// Cmd+N: an empty file with no name yet; saving asks where to put it.
    fn new_untitled(&mut self, _: &NewUntitled, window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.new(|cx| Editor::new(Default::default(), None, cx));
        self.add_tab(editor, window, cx);
    }

    /// Save from anywhere in the window: the open file is saved even when the
    /// keyboard is in the tree or the terminal.
    fn save_active(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |editor, cx| editor.save_from_keyboard(cx));
        }
    }

    fn save_all(&mut self, _: &SaveAll, _: &mut Window, cx: &mut Context<Self>) {
        let dirty: Vec<Entity<Editor>> =
            self.tabs.iter().map(|t| t.editor.clone()).filter(|e| e.read(cx).buffer.is_dirty()).collect();
        let count = dirty.len();
        let saved = dirty.into_iter().filter(|e| e.update(cx, |e, cx| e.save_to_disk(cx))).count();
        if count > 1 {
            self.show_notice(format!("Saved {saved} of {count} files"), cx);
        }
    }

    fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor().cloned() {
            self.ask_where_to_save(editor, window, cx);
        }
    }

    /// Asks for a location, then saves the editor there.
    fn ask_where_to_save(&mut self, editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        let current = editor.read(cx).path().map(Path::to_path_buf);
        let dir = current
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.tree.read(cx).root().to_path_buf());
        let name = current.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let answer = cx.prompt_for_new_path(&dir, name.as_deref());
        let lsp = self.lsp.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = answer.await else { return };
            this.update_in(cx, |this, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.set_path(path.clone(), Some(lsp), cx);
                    editor.save_to_disk(cx);
                });
                if let Some(ix) = this.tabs.iter().position(|t| t.editor == editor) {
                    this.activate(ix, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Quits, after asking about unsaved changes (finishing the close quits).
    pub fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_unsaved(CloseAction::Quit, window, cx);
    }

    fn open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update_in(cx, |this, window, cx| this.open_path(path, window, cx)).ok();
        })
        .detach();
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.close_tab_at(ix, window, cx);
        }
    }

    /// The next tab on the same side, round to the first.
    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(1, window, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(-1, window, cx);
    }

    fn step_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else { return };
        let side = self.side_tabs(self.tabs[ix].side);
        let at = side.iter().position(|&i| i == ix).unwrap_or(0) as isize;
        let next = side[(at + step).rem_euclid(side.len() as isize) as usize];
        self.activate(next, window, cx);
    }

    /// Every command the palette offers right now, by category, with its shortcut.
    fn commands(&self, window: &Window, cx: &App) -> Vec<Command> {
        use Category::*;
        let settings = cx.global::<Settings>();
        let current = |on: bool| if on { " (current)" } else { "" };
        let ai_label = |id: ProviderId| format!("Use {}{}", id.label(), current(settings.ai.provider == id));
        let theme_label = |name: ThemeName| format!("{} Theme{}", name.label(), current(settings.theme == name));
        let toggle = |on: bool, stop: &str, start: &str| if on { stop.to_string() } else { start.to_string() };
        let mut commands: Vec<(Category, String, Box<dyn Action>)> = vec![
            (File, "New File".into(), Box::new(NewUntitled)),
            (File, "New File in Project…".into(), Box::new(crate::file_tree::NewFile)),
            (File, "New Folder in Project…".into(), Box::new(crate::file_tree::NewFolder)),
            (File, "Open File or Folder…".into(), Box::new(Open)),
            (File, "Reopen Closed Tab".into(), Box::new(ReopenClosedTab)),
            (Go, "Go to File…".into(), Box::new(TogglePalette)),
            (Go, "Go to Symbol in Project…".into(), Box::new(GoToSymbolInProject)),
            (Go, "Search in Project…".into(), Box::new(SearchProject)),
            (Edit, "Replace in Project…".into(), Box::new(ReplaceInProject)),
            (View, "Show Files".into(), Box::new(ShowFiles)),
            (View, toggle(settings.word_wrap, "Stop Wrapping Lines", "Wrap Lines"), Box::new(ToggleWordWrap)),
            (
                View,
                toggle(settings.fade_bars_while_typing, "Stop Fading Bars While Typing", "Fade Bars While Typing"),
                Box::new(ToggleFadeWhileTyping),
            ),
            (Appearance, theme_label(ThemeName::Oled), Box::new(UseOledTheme)),
            (Appearance, theme_label(ThemeName::Graphite), Box::new(UseGraphiteTheme)),
            (Appearance, theme_label(ThemeName::Paper), Box::new(UsePaperTheme)),
            (Appearance, "Bigger Text".into(), Box::new(IncreaseFontSize)),
            (Appearance, "Smaller Text".into(), Box::new(DecreaseFontSize)),
            (Appearance, "Actual Size".into(), Box::new(ResetFontSize)),
            (
                Edit,
                toggle(settings.autocomplete, "Turn Off Autocomplete", "Turn On Autocomplete"),
                Box::new(ToggleAutocomplete),
            ),
            (App, "Settings…".into(), Box::new(OpenSettings)),
            (View, "Toggle Sidebar".into(), Box::new(ToggleSidebar)),
            (View, "Toggle Terminal".into(), Box::new(ToggleTerminal)),
            (App, "Edit Settings as JSON".into(), Box::new(OpenSettingsFile)),
        ];
        if self.active.is_some() {
            commands.extend([
                (File, "Save".into(), Box::new(Save) as Box<dyn Action>),
                (File, "Save As…".into(), Box::new(SaveAs)),
                (File, "Save All".into(), Box::new(SaveAll)),
                (File, "Close Tab".into(), Box::new(CloseTab)),
                (File, "Close All Tabs".into(), Box::new(CloseAllTabs)),
                (File, "Close Other Tabs".into(), Box::new(CloseOtherTabs)),
                (Edit, "Undo".into(), Box::new(Undo)),
                (Edit, "Redo".into(), Box::new(Redo)),
                (Edit, "Select All".into(), Box::new(SelectAll)),
                (Edit, "Find…".into(), Box::new(DeployFind)),
                (Edit, "Replace…".into(), Box::new(DeployReplace)),
                (Lines, "Toggle Comment".into(), Box::new(crate::editor::ToggleComment)),
                (Lines, "Indent (Tab on a selection)".into(), Box::new(crate::editor::Indent)),
                (Lines, "Outdent (Shift+Tab)".into(), Box::new(crate::editor::Outdent)),
                (Lines, "Move Line Up".into(), Box::new(crate::editor::MoveLineUp)),
                (Lines, "Move Line Down".into(), Box::new(crate::editor::MoveLineDown)),
                (Lines, "Duplicate Line".into(), Box::new(crate::editor::DuplicateLineDown)),
                (Lines, "Delete Line".into(), Box::new(crate::editor::DeleteLine)),
                (Lines, "Select Line".into(), Box::new(crate::editor::SelectLine)),
                (Lines, "Indent with Tabs".into(), Box::new(crate::editor::IndentWithTabs)),
                (Lines, "Indent with 2 Spaces".into(), Box::new(crate::editor::IndentWith2Spaces)),
                (Lines, "Indent with 4 Spaces".into(), Box::new(crate::editor::IndentWith4Spaces)),
                (Lines, "Use LF Line Endings (macOS, Linux)".into(), Box::new(crate::editor::UseLfLineEndings)),
                (Lines, "Use CRLF Line Endings (Windows)".into(), Box::new(crate::editor::UseCrlfLineEndings)),
                (Lines, "Fold".into(), Box::new(crate::editor::Fold)),
                (Lines, "Unfold".into(), Box::new(crate::editor::Unfold)),
                (Lines, "Fold All".into(), Box::new(crate::editor::FoldAll)),
                (Lines, "Unfold All".into(), Box::new(crate::editor::UnfoldAll)),
                (Cursors, "Add Next Occurrence".into(), Box::new(crate::editor::AddNextOccurrence)),
                (Cursors, "Select All Occurrences".into(), Box::new(crate::editor::SelectAllOccurrences)),
                (Cursors, "Add Cursor Above".into(), Box::new(crate::editor::AddCursorAbove)),
                (Cursors, "Add Cursor Below".into(), Box::new(crate::editor::AddCursorBelow)),
                (Go, "Go to Line…".into(), Box::new(GoToLine)),
                (Go, "Go to Symbol…".into(), Box::new(GoToSymbol)),
                (Go, "Go to Definition".into(), Box::new(GoToDefinition)),
                (Go, "Find References".into(), Box::new(crate::editor::FindReferences)),
                (Go, "Show Problems".into(), Box::new(ShowProblems)),
                (Edit, "Rename Symbol".into(), Box::new(crate::editor::RenameSymbol)),
                (Edit, "Format Document".into(), Box::new(crate::editor::FormatDocument)),
                (
                    Edit,
                    toggle(settings.format_on_save, "Stop Formatting on Save", "Format on Save"),
                    Box::new(ToggleFormatOnSave),
                ),
                (Go, "Show Info at Cursor".into(), Box::new(ShowInfo)),
                (Go, "Next Tab".into(), Box::new(NextTab)),
                (View, "Move Tab to the Right Side".into(), Box::new(MoveTabRight)),
                (View, "Move Tab to the Left Side".into(), Box::new(MoveTabLeft)),
                (Go, "Previous Tab".into(), Box::new(PreviousTab)),
            ]);
        }
        // With AI switched off, nothing about it shows up.
        if settings.ai.enabled {
            commands.extend([
                (Ai, "Ask About This File…".into(), Box::new(AskAi) as Box<dyn Action>),
                (Ai, ai_label(ProviderId::Nvidia), Box::new(UseNvidia)),
                (Ai, ai_label(ProviderId::Ollama), Box::new(UseOllama)),
                (Ai, ai_label(ProviderId::OpenaiCompatible), Box::new(UseOpenAiCompatible)),
                (Ai, ai_label(ProviderId::Claude), Box::new(UseClaudeApi)),
                (Ai, ai_label(ProviderId::ClaudeCode), Box::new(UseClaudeCode)),
                (Ai, ai_label(ProviderId::Codex), Box::new(UseCodex)),
                (Ai, "Set API Key…".into(), Box::new(SetApiKey)),
            ]);
            if self.active.is_some() {
                commands.push((Ai, "Edit with AI…".into(), Box::new(crate::editor::InlineAssist)));
            }
            match self.ai_task.as_ref().map(|t| &t.state) {
                None => commands.push((Ai, "New AI Task…".into(), Box::new(NewAiTask))),
                Some(TaskState::Review(_)) => commands.extend([
                    (Ai, "Review AI Task Changes…".into(), Box::new(ReviewAiTask) as Box<dyn Action>),
                    (Ai, "Keep All AI Task Changes".into(), Box::new(KeepAllTaskChanges)),
                    (Ai, "Undo All AI Task Changes".into(), Box::new(UndoAllTaskChanges)),
                ]),
                Some(_) => commands.push((Ai, "Stop AI Task".into(), Box::new(StopAiTask))),
            }
        }
        commands.push((App, "Welcome to Null…".into(), Box::new(ShowWelcome)));
        commands.push((App, "Quit Null".into(), Box::new(Quit)));
        commands
            .into_iter()
            .map(|(category, label, action)| Command {
                category,
                label: label.into(),
                keys: window.highest_precedence_binding_for_action(action.as_ref()).map(|b| format_keys(&b)),
                action,
            })
            .collect()
    }

    /// Opens the palette of that kind. Pressing the same shortcut again closes it;
    /// another one switches to that kind.
    pub fn open_palette(&mut self, kind: PaletteKind, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette_with(kind, None, Vec::new(), window, cx);
    }

    /// A list of places (references, problems), shown like the palette.
    fn open_locations(
        &mut self,
        title: String,
        locations: Vec<crate::palette::Location>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Replacing a list keeps the way back to where the keyboard was before it.
        let replacing = self.palette.take().is_some();
        let saved = self.focus_before_palette.clone();
        self.open_palette_with(PaletteKind::Locations, Some(title), locations, window, cx);
        if replacing {
            self.focus_before_palette = saved;
        }
    }

    fn open_palette_with(
        &mut self,
        kind: PaletteKind,
        title: Option<String>,
        locations: Vec<crate::palette::Location>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.welcome.is_some() {
            return;
        }
        if let Some((palette, _)) = &self.palette {
            if palette.read(cx).kind() == kind {
                return self.close_palette(window, cx);
            }
            self.palette = None;
        } else if self.settings_panel.take().is_none() {
            // The palette replaces the Settings window, if it's open.
            self.focus_before_palette = window.focused(cx);
        }
        let active_path = self.active_editor().and_then(|e| e.read(cx).path().map(Path::to_path_buf));
        let options = PaletteOptions {
            kind,
            commands: if kind == PaletteKind::Quick {
                self.commands(window, cx)
            } else {
                std::mem::take(&mut self.pending_commands)
            },
            root: self.tree.read(cx).root().to_path_buf(),
            recent_files: self.recent_files.iter().filter(|p| Some(*p) != active_path.as_ref()).cloned().collect(),
            recent_commands: self.recent_commands.clone(),
            line_count: self.active_editor().map(|e| e.read(cx).buffer.len_lines()),
            terminal_open: self.terminal_open.on,
            title,
            locations,
        };
        let palette = cx.new(|cx| Palette::new(options, cx));
        let subscription = cx.subscribe_in(&palette, window, |this, palette, event, window, cx| match event {
            PaletteEvent::Dismissed => this.close_palette(window, cx),
            PaletteEvent::StartTask(text) => {
                let text = text.clone();
                this.close_palette(window, cx);
                this.start_task(text, window, cx);
            }
            PaletteEvent::OpenFile(path) => {
                let path = path.clone();
                this.close_palette(window, cx);
                this.open_file(path, window, cx);
            }
            PaletteEvent::OpenLocation(path, position) => {
                let (path, position) = (path.clone(), *position);
                // Going there: no going back to the view before the preview.
                this.view_before_preview = None;
                this.close_palette(window, cx);
                if this.review_file(&path, window, cx) {
                    return;
                }
                this.open_file(path, window, cx);
                if let Some(editor) = this.active_editor() {
                    let range = lsp_types::Range { start: position, end: position };
                    editor.update(cx, |editor, cx| editor.select_lsp_range(range, cx));
                }
            }
            PaletteEvent::Preview(path, position) => {
                if let Some(editor) = this
                    .active_editor()
                    .filter(|e| e.read(cx).path() == Some(path.as_path()) || e.read(cx).path().is_none())
                {
                    let range = lsp_types::Range { start: *position, end: *position };
                    editor.update(cx, |editor, cx| editor.preview_lsp_range(range, cx));
                }
            }
            PaletteEvent::GoToLine(line) => {
                let line = *line;
                this.close_palette(window, cx);
                if let Some(editor) = this.active_editor() {
                    editor.update(cx, |editor, cx| editor.go_to_line(line, cx));
                }
            }
            PaletteEvent::Run(action) => {
                let action = action.boxed_clone();
                this.remember_command(action.name());
                this.close_palette(window, cx);
                // Run once focus is back where it was, so editor commands reach the editor.
                window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
            }
            PaletteEvent::Apply(action) => {
                // A quick setting: change it and stay in the palette to see the effect.
                let action = action.boxed_clone();
                let palette = palette.clone();
                let workspace = cx.entity();
                window.defer(cx, move |window, cx| {
                    window.dispatch_action(action, cx);
                    // Opening the terminal takes focus; give it back to the palette.
                    window.focus(&palette.focus_handle(cx));
                    let open = workspace.read(cx).terminal_open.on;
                    palette.update(cx, |palette, cx| palette.set_terminal_open(open, cx));
                });
            }
        });
        window.focus(&palette.focus_handle(cx));
        self.palette = Some((palette, subscription));
        cx.notify();
    }

    fn remember_command(&mut self, name: &'static str) {
        self.recent_commands.retain(|n| *n != name);
        self.recent_commands.insert(0, name);
        self.recent_commands.truncate(20);
    }

    fn show_commands(&mut self, _: &ShowCommands, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteKind::Quick, window, cx);
    }

    /// ⌘K with something already typed: the status bar's indentation opens on "indent".
    fn show_commands_for(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
        }
        self.open_palette(PaletteKind::Quick, window, cx);
        if let Some((palette, _)) = &self.palette {
            palette.update(cx, |palette, cx| palette.set_query(query, cx));
        }
    }

    fn ask_ai(&mut self, _: &AskAi, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            editor.update(cx, |editor, cx| editor.ask_inline(window, cx));
        }
    }

    fn toggle_ai(&mut self, _: &ToggleAi, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.ai.enabled = !s.ai.enabled);
        let ai = &cx.global::<Settings>().ai;
        if ai.enabled && ai.provider == ProviderId::Off {
            let keys = crate::palette::shortcut(&OpenSettings, cx).map(|k| format!(" ({k})")).unwrap_or_default();
            self.show_notice(format!("AI is on. Choose where answers come from in Settings → AI{keys}."), cx);
        }
    }

    fn toggle_palette(&mut self, _: &TogglePalette, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteKind::Files, window, cx);
    }

    fn go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_editor().is_none() {
            return self.show_notice("Open a file to go to one of its lines".into(), cx);
        }
        self.open_palette(PaletteKind::Line, window, cx);
    }

    /// Closes whichever floating layer is open (the palette or the Settings window)
    /// and puts focus back where it was.
    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Leaving a symbol list without choosing: the view goes back to where it was.
        if let Some((editor, (line, column, top))) = self.view_before_preview.take() {
            editor.update(cx, |editor, cx| editor.restore_view(line, column, top, cx));
        }
        self.palette = None;
        self.settings_panel = None;
        let fallback = self.active_editor().map(|e| e.focus_handle(cx)).unwrap_or(self.focus_handle.clone());
        window.focus(&self.focus_before_palette.take().unwrap_or(fallback));
        cx.notify();
    }

    /// Brings the window in line with settings after they change.
    fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let settings = cx.global::<Settings>().clone();
        self.sidebar.set(settings.sidebar_visible, SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        if !settings.fade_bars_while_typing {
            self.chrome.set(true, FADE_IN, FADE_OUT);
        }
        for tab in &self.tabs {
            tab.editor.update(cx, |editor, cx| {
                editor.set_font_size(px(settings.font_size), cx);
                // Wrapping on or off moves everything: keep the caret in view.
                editor.autoscroll = true;
                cx.notify();
            });
        }
        menus::set(cx);
        cx.notify();
    }

    fn search_project(&mut self, _: &SearchProject, window: &mut Window, cx: &mut Context<Self>) {
        let selected =
            self.active_editor().map(|e| e.read(cx).selected_text()).filter(|t| !t.is_empty() && !t.contains('\n'));
        self.sidebar_search.set(true, SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        settings::update(cx, |s| s.sidebar_visible = true);
        self.project_search.update(cx, |search, cx| search.focus(selected, window, cx));
        cx.notify();
    }

    /// ⌘⇧H: project search with its replace field open.
    fn replace_in_project(&mut self, _: &ReplaceInProject, window: &mut Window, cx: &mut Context<Self>) {
        let selected =
            self.active_editor().map(|e| e.read(cx).selected_text()).filter(|t| !t.is_empty() && !t.contains('\n'));
        self.sidebar_search.set(true, SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        settings::update(cx, |s| s.sidebar_visible = true);
        self.project_search.update(cx, |search, cx| search.show_replace(selected, window, cx));
        cx.notify();
    }

    /// Tells project search about a tab's unsaved text (or that it's saved now).
    fn share_unsaved(&self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let editor = editor.read(cx);
        let Some(path) = editor.path().map(Path::to_path_buf) else { return };
        let text = editor.buffer.is_dirty().then(|| editor.buffer.rope().clone());
        let unsaved = &mut cx.default_global::<crate::project_search::UnsavedFiles>().0;
        match text {
            Some(text) => unsaved.insert(path, text),
            None => unsaved.remove(&path),
        };
    }

    // ---------- AI tasks ----------

    /// ⌥⌘I: describe a bigger job for Claude Code or Codex.
    fn new_ai_task(&mut self, _: &NewAiTask, window: &mut Window, cx: &mut Context<Self>) {
        let provider = cx.global::<Settings>().ai.active();
        if !crate::ai::can_run_tasks(provider) {
            return self.show_notice("Tasks need AI: choose where it comes from in Settings → AI.".into(), cx);
        }
        match self.ai_task.as_ref().map(|t| &t.state) {
            Some(TaskState::Starting | TaskState::Running(_)) => {
                return self.show_notice("A task is already running.".into(), cx);
            }
            Some(TaskState::Review(_)) => {
                self.show_notice("First review what the last task changed.".into(), cx);
                return self.review_ai_task(&ReviewAiTask, window, cx);
            }
            None => {}
        }
        self.open_palette_with(PaletteKind::Task, Some(provider.label().into()), Vec::new(), window, cx);
    }

    fn start_task(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        let root = self.tree.read(cx).root().to_path_buf();
        // The task reads and writes files on disk: unsaved edits go there first.
        let mut saved = 0;
        for tab in &self.tabs {
            let dirty = tab.editor.read(cx).buffer.is_dirty() && tab.editor.read(cx).path().is_some();
            if dirty && tab.editor.update(cx, |editor, cx| editor.save_to_disk(cx)) {
                saved += 1;
            }
        }
        if saved > 0 {
            let files = if saved == 1 { "1 file".to_string() } else { format!("{saved} files") };
            self.show_notice(format!("Saved {files} so the task sees them."), cx);
        }
        let title: String = text.chars().take(40).collect();
        let child: Arc<std::sync::Mutex<Option<std::process::Child>>> = Arc::default();
        let stop: Arc<std::sync::atomic::AtomicBool> = Arc::default();
        let settings = cx.global::<Settings>().ai.clone();
        let running = child.clone();
        let stopping = stop.clone();
        let task = cx.spawn(async move |this, cx| {
            let snapshot = {
                let root = root.clone();
                cx.background_executor().spawn(async move { crate::ai_task::Snapshot::take(&root) }).await
            };
            this.update(cx, |this, cx| {
                if let Some(run) = &mut this.ai_task {
                    run.state = TaskState::Running(None);
                }
                cx.notify();
            })
            .ok();
            let (tx, mut rx) = futures::channel::mpsc::unbounded();
            let work = {
                let root = root.clone();
                cx.background_executor().spawn(async move {
                    crate::ai::run_task(&settings, &root, &text, &running, &stopping, &mut |event| {
                        tx.unbounded_send(event).ok();
                    })
                })
            };
            use futures::StreamExt;
            while let Some(crate::ai::TaskEvent::File(file)) = rx.next().await {
                this.update(cx, |this, cx| {
                    if let Some(run) = &mut this.ai_task {
                        run.state = TaskState::Running(Some(file));
                    }
                    cx.notify();
                })
                .ok();
            }
            let result = work.await;
            // Whatever happened (done, failed, stopped), see what changed.
            let changes = cx.background_executor().spawn(async move { snapshot.changes(&root) }).await;
            this.update(cx, |this, cx| this.task_finished(result, changes, cx)).ok();
        });
        self.ai_task = Some(AiTaskRun { title, state: TaskState::Starting, child, stop, _task: Some(task) });
        cx.notify();
    }

    fn task_finished(
        &mut self,
        result: Result<String, String>,
        changes: Vec<crate::ai_task::FileChange>,
        cx: &mut Context<Self>,
    ) {
        let count = changes.len();
        let files = if count == 1 { "1 file".to_string() } else { format!("{count} files") };
        let message = match (&result, count) {
            (Ok(summary), 0) => format!("The task changed nothing. {summary}"),
            (Ok(summary), _) => format!("Changed {files}: review them from the status bar. {summary}"),
            (Err(error), 0) => error.clone(),
            (Err(error), _) => format!("{error} It changed {files} before stopping: review them from the status bar."),
        };
        self.show_notice(message.trim().to_string(), cx);
        if count == 0 {
            self.ai_task = None;
        } else if let Some(run) = &mut self.ai_task {
            run.state = TaskState::Review(changes);
            run._task = None;
        }
        cx.notify();
    }

    fn stop_ai_task(&mut self, _: &StopAiTask, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(run) = &self.ai_task {
            run.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(mut child) = run.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
                child.kill().ok();
            }
        }
        cx.notify();
    }

    /// The files the task changed, to open one by one; then keep or undo them all.
    fn review_ai_task(&mut self, _: &ReviewAiTask, window: &mut Window, cx: &mut Context<Self>) {
        use crate::palette::{Category, Command, Location, LocationKind};
        let Some(AiTaskRun { state: TaskState::Review(changes), title, .. }) = &self.ai_task else { return };
        let root = self.tree.read(cx).root().to_path_buf();
        let locations = changes
            .iter()
            .map(|c| Location {
                path: c.path.clone(),
                position: lsp_types::Position::default(),
                text: c.path.strip_prefix(&root).unwrap_or(&c.path).display().to_string(),
                kind: LocationKind::FileChange(c.kind, c.added, c.removed),
            })
            .collect();
        let title = format!("✦ {title}");
        self.pending_commands = vec![
            Command {
                category: Category::Ai,
                label: "Keep All Changes".into(),
                action: Box::new(KeepAllTaskChanges),
                keys: None,
            },
            Command {
                category: Category::Ai,
                label: "Undo All Changes".into(),
                action: Box::new(UndoAllTaskChanges),
                keys: None,
            },
        ];
        self.open_locations(title, locations, window, cx);
    }

    /// Opening a file from the review list: it opens with the task's changes to keep or
    /// undo. A deleted file is put back. False when the file isn't part of the task.
    fn review_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(AiTaskRun { state: TaskState::Review(changes), .. }) = &mut self.ai_task else { return false };
        let Some(index) = changes.iter().position(|c| c.path == path) else { return false };
        let change = changes[index].clone();
        if change.kind == crate::ai_task::ChangeKind::Deleted {
            let message = match crate::ai_task::undo(&change) {
                Ok(()) => format!("Restored {}", change.path.file_name().unwrap_or_default().to_string_lossy()),
                Err(error) => format!("Couldn't restore it: {error}"),
            };
            changes.remove(index);
            self.show_notice(message, cx);
            self.end_task_if_reviewed(cx);
            return true;
        }
        self.open_file(path.to_path_buf(), window, cx);
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |editor, cx| {
                // The file as it is on disk now, whatever the tab had.
                editor.reload_from_disk(cx);
                editor.start_review(change.before.clone(), cx);
            });
            if !editor.read(cx).in_review() {
                self.file_reviewed(&editor, cx);
            }
        }
        true
    }

    /// Every change in a file was kept or undone: it leaves the list.
    fn file_reviewed(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let Some(path) = editor.read(cx).path().map(Path::to_path_buf) else { return };
        if let Some(AiTaskRun { state: TaskState::Review(changes), .. }) = &mut self.ai_task {
            changes.retain(|c| c.path != path);
        }
        self.end_task_if_reviewed(cx);
    }

    fn end_task_if_reviewed(&mut self, cx: &mut Context<Self>) {
        if matches!(&self.ai_task, Some(AiTaskRun { state: TaskState::Review(changes), .. }) if changes.is_empty()) {
            self.ai_task = None;
            self.show_notice("Every change reviewed.".into(), cx);
        }
        cx.notify();
    }

    fn keep_all_task_changes(&mut self, _: &KeepAllTaskChanges, _: &mut Window, cx: &mut Context<Self>) {
        self.ai_task = None;
        for tab in &self.tabs {
            tab.editor.update(cx, |editor, cx| editor.end_review(cx));
        }
        self.show_notice("Kept every change.".into(), cx);
    }

    /// Every file back as it was before the task (an added file goes to the Trash).
    fn undo_all_task_changes(&mut self, _: &UndoAllTaskChanges, _: &mut Window, cx: &mut Context<Self>) {
        let Some(AiTaskRun { state: TaskState::Review(changes), .. }) = self.ai_task.take() else { return };
        let mut failed = Vec::new();
        for change in &changes {
            let open = self.tabs.iter().find(|t| t.editor.read(cx).path() == Some(change.path.as_path()));
            match (open.map(|t| t.editor.clone()), change.kind) {
                // An open file goes back in its editor too (one undo step), then to disk.
                (Some(editor), crate::ai_task::ChangeKind::Changed) => {
                    editor.update(cx, |editor, cx| {
                        editor.end_review(cx);
                        let all = 0..editor.buffer.len_chars();
                        editor.apply_char_edits(vec![(all, change.before.clone())], cx);
                        editor.save_to_disk(cx);
                    });
                }
                _ => {
                    if let Err(error) = crate::ai_task::undo(change) {
                        failed.push(format!("{}: {error}", change.path.display()));
                    }
                }
            }
        }
        let message = if failed.is_empty() {
            "Every file is back as it was before the task.".to_string()
        } else {
            format!("Couldn't undo everything: {}", failed.join("; "))
        };
        self.show_notice(message, cx);
        cx.notify();
    }

    /// Replaces a search's matches across files. Files open in a tab change in their
    /// editor (one undo step each, saved when you save); the others are written on disk.
    fn replace_in_files(
        &mut self,
        query: &crate::search::SearchQuery,
        replacement: &str,
        targets: &[(PathBuf, Option<usize>)],
        cx: &mut Context<Self>,
    ) {
        let (mut replaced, mut files, mut failed) = (0, 0, Vec::new());
        for (path, line) in targets {
            let open =
                self.tabs.iter().find(|t| t.editor.read(cx).path() == Some(path.as_path())).map(|t| t.editor.clone());
            match open {
                Some(editor) => {
                    let count = editor.update(cx, |editor, cx| {
                        let text = editor.buffer.to_string();
                        let rope = editor.buffer.rope().clone();
                        let edits: Vec<_> = crate::project_search::replacements(&text, query, replacement, *line)
                            .into_iter()
                            .map(|(r, new)| (rope.byte_to_char(r.start)..rope.byte_to_char(r.end), new))
                            .collect();
                        let count = edits.len();
                        editor.apply_char_edits(edits, cx);
                        count
                    });
                    // Before the search runs again, so it sees the new text.
                    self.share_unsaved(&editor, cx);
                    replaced += count;
                    files += (count > 0) as usize;
                }
                None => {
                    let Ok(text) = std::fs::read_to_string(path) else { continue };
                    let edits = crate::project_search::replacements(&text, query, replacement, *line);
                    if edits.is_empty() {
                        continue;
                    }
                    let mut new = text.clone();
                    for (range, with) in edits.iter().rev() {
                        new.replace_range(range.clone(), with);
                    }
                    match std::fs::write(path, new) {
                        Ok(()) => {
                            replaced += edits.len();
                            files += 1;
                        }
                        Err(_) => {
                            failed.push(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                        }
                    }
                }
            }
        }
        let matches = if replaced == 1 { "1 match".to_string() } else { format!("{replaced} matches") };
        let files = if files == 1 { "1 file".to_string() } else { format!("{files} files") };
        let mut message = format!("Replaced {matches} in {files}");
        if !failed.is_empty() {
            message.push_str(&format!(". Couldn't write {}", failed.join(", ")));
        }
        self.show_notice(message, cx);
        self.project_search.update(cx, |search, cx| search.refresh(cx));
    }

    fn show_files(&mut self, _: &ShowFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_search.set(false, SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        settings::update(cx, |s| s.sidebar_visible = true);
        self.leave_hidden_focus(window, cx);
        cx.notify();
    }

    /// When the focused field is about to disappear (the search box when switching
    /// to files, or the sidebar being hidden), focus the text instead. Otherwise
    /// focus would sit on something no longer on screen and shortcuts would stop working.
    fn leave_hidden_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let search_hidden = !self.sidebar_search.on || !cx.global::<Settings>().sidebar_visible;
        if search_hidden && self.project_search.focus_handle(cx).contains_focused(window, cx) {
            self.focus_main(window, cx);
        }
    }

    /// Focuses the open file, or the window itself when nothing is open.
    fn focus_main(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active_editor() {
            Some(editor) => window.focus(&editor.focus_handle(cx)),
            None => window.focus(&self.focus_handle),
        }
    }

    fn render_sidebar_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>();
        let searching = self.sidebar_search.on;
        let tab = |id: &'static str, label: &'static str, on: bool| {
            ui::segment(on, theme)
                .id(id)
                .flex_1()
                .flex()
                .justify_center()
                .cursor_pointer()
                .when(!on, |t| t.hover(|s| s.text_color(theme.foreground)))
                .child(label)
        };
        div()
            .px(px(12.))
            .pt(px(10.))
            .child(
                ui::segmented(theme)
                    .child(tab("files", "Files", !searching).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.show_files(&ShowFiles, window, cx)),
                    ))
                    .child(tab("search", "Search", searching).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.search_project(&SearchProject, window, cx)),
                    )),
            )
            .into_any_element()
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, window: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.sidebar_visible = !s.sidebar_visible);
        self.leave_hidden_focus(window, cx);
    }

    fn toggle_fade_while_typing(&mut self, _: &ToggleFadeWhileTyping, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.fade_bars_while_typing = !s.fade_bars_while_typing);
    }

    fn toggle_word_wrap(&mut self, _: &ToggleWordWrap, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.word_wrap = !s.word_wrap);
    }

    fn use_ai(&mut self, provider: ProviderId, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.ai.provider = provider);
        let hint = if provider.uses_api_key() && crate::ai::known_key(provider) == Some(false) {
            " Add a key with “AI: Set API Key”."
        } else {
            ""
        };
        self.show_notice(format!("AI answers now come from {}.{hint}", provider.label()), cx);
    }

    fn use_nvidia(&mut self, _: &UseNvidia, _: &mut Window, cx: &mut Context<Self>) {
        self.use_ai(ProviderId::Nvidia, cx);
    }

    fn use_ollama(&mut self, _: &UseOllama, _: &mut Window, cx: &mut Context<Self>) {
        self.use_ai(ProviderId::Ollama, cx);
    }

    fn use_openai_compatible(&mut self, _: &UseOpenAiCompatible, _: &mut Window, cx: &mut Context<Self>) {
        self.use_ai(ProviderId::OpenaiCompatible, cx);
    }

    fn use_claude_api(&mut self, _: &UseClaudeApi, _: &mut Window, cx: &mut Context<Self>) {
        self.use_ai(ProviderId::Claude, cx);
    }

    fn use_claude_code(&mut self, _: &UseClaudeCode, _: &mut Window, cx: &mut Context<Self>) {
        self.use_ai(ProviderId::ClaudeCode, cx);
    }

    fn use_codex(&mut self, _: &UseCodex, _: &mut Window, cx: &mut Context<Self>) {
        self.use_ai(ProviderId::Codex, cx);
    }

    fn turn_off_ai(&mut self, _: &TurnOffAi, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.ai.provider = ProviderId::Off);
        self.show_notice("AI is off. Nothing AI-related will appear.".into(), cx);
    }

    fn set_api_key(&mut self, _: &SetApiKey, window: &mut Window, cx: &mut Context<Self>) {
        let provider = cx.global::<Settings>().ai.provider;
        if !provider.uses_api_key() {
            let message = match provider {
                ProviderId::Off => "Choose a provider first with “AI: Use …”.".to_string(),
                _ => format!("{} doesn't use an API key.", provider.label()),
            };
            return self.show_notice(message, cx);
        }
        self.focus_before_palette = window.focused(cx);
        let prompt = cx.new(|cx| KeyPrompt::new(provider, cx));
        let subscription = cx.subscribe_in(&prompt, window, |this, _, event, window, cx| {
            if let KeyPromptEvent::Finished(message) = event {
                this.show_notice(message.clone(), cx);
            }
            this.key_prompt = None;
            this.close_palette(window, cx);
        });
        window.focus(&prompt.focus_handle(cx));
        self.key_prompt = Some((prompt, subscription));
        cx.notify();
    }

    fn show_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.notice = Some((message, Instant::now()));
        self.notice_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            this.update(cx, |this, cx| {
                this.notice = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn toggle_terminal(&mut self, _: &ToggleTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_open.on {
            let had_focus = self.terminal.as_ref().is_some_and(|(t, _)| t.focus_handle(cx).is_focused(window));
            self.terminal_open.set(false, TERMINAL_SLIDE, TERMINAL_SLIDE);
            if had_focus {
                self.focus_main(window, cx);
            }
            return cx.notify();
        }
        if self.terminal.is_none() {
            let root = self.tree.read(cx).root().to_path_buf();
            let shell = match Shell::start(root) {
                Ok(shell) => shell,
                Err(err) => return self.show_notice(format!("Couldn't start a terminal: {err}"), cx),
            };
            let terminal = cx.new(|cx| TerminalView::new(shell, cx));
            let subscription = cx.subscribe_in(&terminal, window, |this, _, event, window, cx| match event {
                TerminalEvent::TitleChanged => cx.notify(),
                // The shell exited (e.g. `exit`): close the panel; the next toggle starts a new one.
                TerminalEvent::Exited => {
                    this.terminal = None;
                    this.terminal_open.set(false, TERMINAL_SLIDE, TERMINAL_SLIDE);
                    this.focus_main(window, cx);
                    cx.notify();
                }
            });
            self.terminal = Some((terminal, subscription));
        }
        self.terminal_open.set(true, TERMINAL_SLIDE, TERMINAL_SLIDE);
        if let Some((terminal, _)) = &self.terminal {
            window.focus(&terminal.focus_handle(cx));
        }
        cx.notify();
    }

    fn render_terminal_panel(&self, height: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (terminal, _) = self.terminal.as_ref()?;
        if height < 0.5 {
            return None;
        }
        let theme = cx.global::<Theme>();
        let title = terminal.read(cx).title.clone();
        let header = div()
            .h(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .text_size(px(ui::T_SM))
            .text_color(theme.muted)
            .child(div().text_color(theme.foreground).child("Terminal"))
            .child(div().flex_1().min_w_0().truncate().text_color(theme.muted).child(title))
            .child(
                div()
                    .id("close-terminal")
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(ui::R_KEY))
                    .cursor_pointer()
                    .group("close-terminal")
                    .hover(|s| s.bg(theme.hairline))
                    .child(
                        svg()
                            .path("icons/x.svg")
                            .size(px(12.))
                            .text_color(theme.muted)
                            .group_hover("close-terminal", |s| s.text_color(theme.foreground)),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_terminal(&ToggleTerminal, window, cx)
                    })),
            );
        Some(
            div()
                .flex_none()
                .h(px(height))
                .overflow_hidden()
                .border_t_1()
                .border_color(theme.hairline)
                .bg(theme.background)
                .child(
                    div()
                        .h(px(TERMINAL_HEIGHT))
                        .flex()
                        .flex_col()
                        .child(header)
                        .child(div().flex_1().min_h_0().child(terminal.clone())),
                )
                .into_any_element(),
        )
    }

    fn toggle_autocomplete(&mut self, _: &ToggleAutocomplete, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.autocomplete = !s.autocomplete);
    }

    fn increase_font_size(&mut self, _: &IncreaseFontSize, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.font_size += 1.);
    }

    fn decrease_font_size(&mut self, _: &DecreaseFontSize, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.font_size -= 1.);
    }

    fn reset_font_size(&mut self, _: &ResetFontSize, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.font_size = DEFAULT_FONT_SIZE);
    }

    fn use_oled_theme(&mut self, _: &UseOledTheme, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.theme = ThemeName::Oled);
    }

    fn use_graphite_theme(&mut self, _: &UseGraphiteTheme, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.theme = ThemeName::Graphite);
    }

    fn use_paper_theme(&mut self, _: &UsePaperTheme, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.theme = ThemeName::Paper);
    }

    /// ⌘, opens the Settings window, or closes it.
    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_at(None, window, cx);
    }

    /// Opens Settings, at `section` if given. With Settings already open, closes it.
    fn open_settings_at(&mut self, section: Option<Section>, window: &mut Window, cx: &mut Context<Self>) {
        if self.welcome.is_some() {
            return;
        }
        if self.settings_panel.is_some() {
            return self.close_palette(window, cx);
        }
        if self.palette.is_some() {
            self.palette = None;
        } else {
            self.focus_before_palette = window.focused(cx);
        }
        let shortcuts = self
            .commands(window, cx)
            .into_iter()
            .map(|c| Shortcut { category: c.category, label: c.label, action: c.action })
            .collect();
        let lsp = self.lsp.clone();
        let panel = cx.new(|cx| {
            let mut panel = SettingsPanel::new(shortcuts, lsp, cx);
            if let Some(section) = section {
                panel.show_section(section);
            }
            panel
        });
        let subscription = cx.subscribe_in(&panel, window, |this, _, event, window, cx| match event {
            SettingsPanelEvent::Closed => this.close_palette(window, cx),
            SettingsPanelEvent::Run(action) => {
                let action = action.boxed_clone();
                this.close_palette(window, cx);
                window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
            }
        });
        window.focus(&panel.focus_handle(cx));
        self.settings_panel = Some((panel, subscription));
        cx.notify();
    }

    /// A line of a file: from its open tab if there is one (it may not be saved), else from disk.
    fn line_text(&self, path: &Path, line: usize, cx: &App) -> String {
        if let Some(tab) = self.tabs.iter().find(|t| t.editor.read(cx).path() == Some(path)) {
            return tab.editor.read(cx).buffer.line_text(line);
        }
        std::fs::read_to_string(path).ok().and_then(|t| t.lines().nth(line).map(str::to_string)).unwrap_or_default()
    }

    /// Applies a rename (or any multi-file change) from a language server: open files as
    /// one undo step each, others written on disk.
    fn apply_workspace_edit(&mut self, edit: lsp_types::WorkspaceEdit, cx: &mut Context<Self>) {
        let mut by_file: Vec<(PathBuf, Vec<lsp_types::TextEdit>)> = Vec::new();
        let mut add = |uri: &lsp_types::Uri, edits: Vec<lsp_types::TextEdit>| {
            let Some(path) = crate::lsp::path_for(uri) else { return };
            match by_file.iter_mut().find(|(p, _)| *p == path) {
                Some((_, list)) => list.extend(edits),
                None => by_file.push((path, edits)),
            }
        };
        for (uri, edits) in edit.changes.unwrap_or_default() {
            add(&uri, edits);
        }
        let operations = match edit.document_changes {
            Some(lsp_types::DocumentChanges::Edits(edits)) => edits,
            Some(lsp_types::DocumentChanges::Operations(ops)) => ops
                .into_iter()
                .filter_map(|op| match op {
                    lsp_types::DocumentChangeOperation::Edit(edit) => Some(edit),
                    lsp_types::DocumentChangeOperation::Op(_) => None,
                })
                .collect(),
            None => Vec::new(),
        };
        for change in operations {
            let edits = change
                .edits
                .into_iter()
                .map(|e| match e {
                    lsp_types::OneOf::Left(edit) => edit,
                    lsp_types::OneOf::Right(annotated) => annotated.text_edit,
                })
                .collect();
            add(&change.text_document.uri, edits);
        }
        let places: usize = by_file.iter().map(|(_, e)| e.len()).sum();
        let files = by_file.len();
        let mut failed = Vec::new();
        for (path, edits) in by_file {
            if let Some(tab) = self.tabs.iter().find(|t| t.editor.read(cx).path() == Some(path.as_path())) {
                tab.editor.update(cx, |editor, cx| editor.apply_lsp_edits(&edits, cx));
            } else {
                let written = std::fs::read_to_string(&path).map(|text| {
                    let mut buffer = crate::buffer::Buffer::from_text(&text);
                    crate::editor::apply_edits(&mut buffer, &edits);
                    std::fs::write(&path, buffer.to_string())
                });
                if !matches!(written, Ok(Ok(()))) {
                    failed.push(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                }
            }
        }
        let message = match (places, failed.is_empty()) {
            (0, _) => "Nothing to rename".to_string(),
            (_, true) if files == 1 => format!("Renamed in {places} places"),
            (_, true) => format!("Renamed in {places} places across {files} files"),
            (_, false) => format!("Renamed, but couldn't write {}", failed.join(", ")),
        };
        self.show_notice(message, cx);
    }

    /// Every error and warning the language servers found, errors first.
    /// The functions, types and constants of a list of definitions, as places to go.
    fn symbol_locations(
        &self,
        definitions: &[crate::project_index::Definition],
        cx: &App,
    ) -> Vec<crate::palette::Location> {
        definitions
            .iter()
            .map(|d| {
                // The caret lands on the name itself, in the columns servers count.
                let line = self.line_text(&d.path, d.row, cx);
                let column = line.find(&d.name).map_or(0, |b| line[..b].encode_utf16().count());
                crate::palette::Location {
                    path: d.path.clone(),
                    position: lsp_types::Position { line: d.row as u32, character: column as u32 },
                    text: d.name.clone(),
                    kind: crate::palette::LocationKind::Symbol(d.kind),
                }
            })
            .collect()
    }

    /// ⌘⇧O: the definitions in the open file, unsaved edits included. Moving through
    /// them shows each in the editor; Esc goes back to where you were.
    fn go_to_symbol(&mut self, _: &GoToSymbol, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else {
            return self.show_notice("Open a file to go to one of its functions or types".into(), cx);
        };
        let (path, text, name) = {
            let e = editor.read(cx);
            (e.path().map(Path::to_path_buf), e.buffer.to_string(), e.file_name())
        };
        let path = path.unwrap_or_else(|| PathBuf::from(&name));
        let definitions = crate::project_index::definitions_in(&path, &text);
        if definitions.is_empty() {
            return self.show_notice(format!("No functions or types found in {name}"), cx);
        }
        // Read from the editor (not the disk) for the columns, since it may be unsaved.
        let locations: Vec<_> = definitions
            .iter()
            .map(|d| {
                let line = editor.read(cx).buffer.line_text(d.row);
                let column = line.find(&d.name).map_or(0, |b| line[..b].encode_utf16().count());
                crate::palette::Location {
                    path: path.clone(),
                    position: lsp_types::Position { line: d.row as u32, character: column as u32 },
                    text: d.name.clone(),
                    kind: crate::palette::LocationKind::Symbol(d.kind),
                }
            })
            .collect();
        let view = editor.read(cx).view_state();
        self.open_locations(format!("Go to a symbol in {name}"), locations, window, cx);
        self.view_before_preview = Some((editor, view));
    }

    /// ⌘T: every definition in the project, by name.
    fn go_to_symbol_in_project(&mut self, _: &GoToSymbolInProject, window: &mut Window, cx: &mut Context<Self>) {
        let Some(definitions) = cx.try_global::<crate::project_index::ProjectContext>().map(|c| c.definitions.clone())
        else {
            return;
        };
        if definitions.is_empty() {
            let message = if self.index_task.is_some() {
                "Still reading the project…"
            } else {
                "No functions or types found in this project"
            };
            return self.show_notice(message.into(), cx);
        }
        let locations = self.symbol_locations(&definitions, cx);
        self.open_locations("Go to a symbol in the project".into(), locations, window, cx);
    }

    fn show_problems(&mut self, _: &ShowProblems, window: &mut Window, cx: &mut Context<Self>) {
        use crate::palette::{Location, LocationKind};
        use lsp_types::DiagnosticSeverity as S;
        let found: Vec<(PathBuf, lsp_types::Diagnostic)> =
            self.lsp.read(cx).all_diagnostics().map(|(p, d)| (p.clone(), d.clone())).collect();
        let mut locations: Vec<Location> = found
            .into_iter()
            .filter_map(|(path, d)| {
                let kind = match d.severity.unwrap_or(S::ERROR) {
                    S::ERROR => LocationKind::Error,
                    S::WARNING => LocationKind::Warning,
                    _ => return None,
                };
                let text = d.message.lines().next().unwrap_or("").to_string();
                Some(Location { path, position: d.range.start, text, kind })
            })
            .collect();
        locations
            .sort_by_key(|l| (l.kind != LocationKind::Error, l.path.clone(), l.position.line, l.position.character));
        let errors = locations.iter().filter(|l| l.kind == LocationKind::Error).count();
        let warnings = locations.len() - errors;
        let plural = |n: usize, word: &str| if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") };
        let title = match (errors, warnings) {
            (0, 0) => "Problems".to_string(),
            (e, 0) => plural(e, "error"),
            (0, w) => plural(w, "warning"),
            (e, w) => format!("{} and {}", plural(e, "error"), plural(w, "warning")),
        };
        if locations.is_empty() {
            return self.show_notice("No problems found".into(), cx);
        }
        self.open_locations(title, locations, window, cx);
    }

    /// The first-launch screen: look, shortcuts and AI.
    pub fn show_welcome(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let welcome = cx.new(Welcome::new);
        let subscription = cx.subscribe_in(&welcome, window, |this, _, WelcomeEvent::Finished, window, cx| {
            this.welcome = None;
            this.focus_main(window, cx);
            cx.notify();
        });
        window.focus(&welcome.focus_handle(cx));
        self.welcome = Some((welcome, subscription));
        cx.notify();
    }

    fn open_settings_file(&mut self, _: &OpenSettingsFile, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = Settings::ensure_file(cx) {
            self.open_file(path, window, cx);
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let moved = self.last_mouse.is_none_or(|last| {
            let d = event.position - last;
            f32::from(d.x).abs() + f32::from(d.y).abs() > WAKE_DISTANCE
        });
        if moved {
            self.last_mouse = Some(event.position);
            if !self.chrome.on {
                self.chrome.set(true, FADE_IN, FADE_OUT);
                cx.notify();
            }
        }
    }

    /// What the status bar says about code intelligence for the open file.
    /// Installs the language server for the open file, or says what's needed first.
    fn install_language_server(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.active_editor().and_then(|e| e.read(cx).path().map(Path::to_path_buf)) else { return };
        let Some(server) = crate::servers::for_path(&path) else { return };
        if let Err(reason) = crate::servers::can_install(server) {
            return self.show_notice(reason, cx);
        }
        self.lsp.update(cx, |lsp, cx| lsp.install(&path, cx));
    }

    fn language_status(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        const TIP_FOR: Duration = Duration::from_secs(8);
        let editor = self.active_editor()?.read(cx);
        let readiness = editor.readiness(cx)?;
        let theme = cx.global::<Theme>();
        let dot = |color| div().size(px(6.)).rounded_full().bg(color);
        let item = |color, text: String| {
            div().flex().items_center().gap(px(6.)).whitespace_nowrap().child(dot(color)).child(text).into_any_element()
        };
        match readiness {
            Readiness::Starting => Some(item(theme.caret, "Starting…".into())),
            Readiness::Indexing { percent } => {
                let percent = percent.map(|p| format!(" {p}%")).unwrap_or_default();
                Some(item(theme.caret, format!("Indexing{percent}")))
            }
            // One quiet line: what's missing, and a word to fix it.
            Readiness::Missing { server } | Readiness::InstallFailed { server } => {
                let failed = matches!(readiness, Readiness::InstallFailed { .. });
                let text = if failed {
                    format!("Couldn't install {}", server.name)
                } else {
                    format!("No {} language server", server.label)
                };
                Some(
                    div()
                        .id("install-server")
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .child(div().text_color(theme.faint).child(text))
                        .child(div().text_color(theme.caret).child(if failed { "Retry" } else { "Install" }))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.install_language_server(cx)))
                        .into_any_element(),
                )
            }
            Readiness::Installing { server } => Some(item(theme.caret, format!("Installing {}…", server.name))),
            Readiness::Unavailable { program } => {
                Some(div().text_color(theme.faint).child(format!("{program} didn't start")).into_any_element())
            }
            Readiness::Ready { checking } => {
                let since = *self.ready_since.get_or_insert_with(|| {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(TIP_FOR).await;
                        this.update(cx, |_, cx| cx.notify()).ok();
                    })
                    .detach();
                    Instant::now()
                });
                if since.elapsed() < TIP_FOR {
                    let tip = if cfg!(target_os = "macos") {
                        "Hold ⌥ over code for info · ⌘-click to jump"
                    } else {
                        "Hold Alt over code for info · Ctrl+click to jump"
                    };
                    return Some(div().text_color(theme.muted).whitespace_nowrap().child(tip).into_any_element());
                }
                checking.then(|| div().text_color(theme.faint).child("Checking…").into_any_element())
            }
        }
    }

    /// Each tab's name, with its folder added where two tabs share a name (mod.rs · a).
    fn tab_labels(&self, cx: &App) -> Vec<(String, Option<String>)> {
        let names: Vec<(String, Option<PathBuf>)> = self
            .tabs
            .iter()
            .map(|t| {
                let editor = t.editor.read(cx);
                (editor.file_name(), editor.path().map(Path::to_path_buf))
            })
            .collect();
        names
            .iter()
            .map(|(name, path)| {
                let shared = names.iter().filter(|(other, _)| other == name).count() > 1;
                let folder = path
                    .as_ref()
                    .filter(|_| shared)
                    .and_then(|p| p.parent()?.file_name())
                    .map(|f| f.to_string_lossy().into_owned());
                (name.clone(), folder)
            })
            .collect()
    }

    /// One side's tabs. The one it shows stands out; most of all on the side being worked in.
    fn render_tabs(&self, side: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let labels = self.tab_labels(cx);
        let split = self.is_split();
        div()
            .id(("tabs", side))
            .flex()
            .gap(px(2.))
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll[side])
            // Dropped past the last tab: at the end of this side.
            .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                this.drop_tab(&dragged.editor, None, side, window, cx)
            }))
            .children(self.tabs.iter().enumerate().filter(|(_, t)| t.side == side).map(|(ix, tab)| {
                let editor = tab.editor.read(cx);
                let shown = self.shown[side].as_ref() == Some(&tab.editor);
                // With two sides, the one not being worked in shows its tab a little quieter.
                let active = shown && (!split || self.active == Some(ix));
                let resting = shown && !active;
                let dirty = editor.buffer.is_dirty();
                let group = format!("tab-{ix}");
                // Unsaved: a small dot, which turns into the close button under the pointer.
                let close = div()
                    .id(("close", ix))
                    .relative()
                    .size(px(16.))
                    .flex_none()
                    .rounded(px(ui::R_KEY))
                    .hover(|s| s.bg(theme.hairline))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(!dirty, |d| d.invisible())
                            .group_hover(group.clone(), |s| s.invisible())
                            .child(div().size(px(7.)).rounded_full().bg(theme.muted)),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(dirty || !active, |d| d.invisible())
                            .group_hover(group.clone(), |s| s.visible())
                            .child(svg().path("icons/x.svg").size(px(10.)).text_color(theme.muted)),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.close_tab_at(ix, window, cx);
                    }));
                let (name, folder) = labels[ix].clone();
                let dragged = DraggedTab { editor: tab.editor.clone(), label: name.clone().into() };
                div()
                    .id(("tab", ix))
                    .group(group)
                    .on_drag(dragged, |tab, _, _, cx| cx.new(|_| TabGhost { label: tab.label.clone() }))
                    .drag_over::<DraggedTab>({
                        let line = theme.caret;
                        move |style, _, _, _| style.border_l_2().border_color(line)
                    })
                    .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                        cx.stop_propagation();
                        this.drop_tab(&dragged.editor, Some(ix), side, window, cx);
                    }))
                    .h(px(28.))
                    .max_w(px(220.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(12.))
                    .pr(px(6.))
                    .rounded(px(ui::R_CONTROL))
                    .text_size(px(ui::T_MD))
                    .text_color(if active || resting { theme.foreground } else { theme.muted })
                    .when(active, |tab| tab.bg(theme.hairline))
                    .when(resting, |tab| tab.bg(theme.hairline.opacity(0.45)))
                    .when(!shown, |tab| tab.hover(|s| s.bg(theme.hairline.opacity(0.6))))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .gap(px(6.))
                            .child(div().min_w_0().truncate().child(name))
                            .children(folder.map(|f| div().flex_none().text_color(theme.faint).child(f))),
                    )
                    .child(close)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.activate(ix, window, cx)))
                    // Middle-click closes, as in browsers.
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| this.close_tab_at(ix, window, cx)),
                    )
            }))
            .into_any_element()
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>();
        // The keys as they're bound now: another preset's, or the person's own.
        let hint = |action: &dyn Action, label: &'static str| {
            let keys = crate::palette::shortcut(action, cx)?;
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().w(px(64.)).flex().justify_end().child(ui::key_cap(keys, theme)))
                    .child(div().text_color(theme.muted).child(label)),
            )
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .text_size(px(ui::T_MD))
            // One column, so the keys and the words line up from row to row.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .children(hint(&Open, "Open a file or folder"))
                    .children(hint(&TogglePalette, "Find a file"))
                    .children(hint(&ToggleSidebar, "Show or hide the files")),
            )
            .into_any_element()
    }
}

/// The status bar's path is cut to about this many characters, from the left.
const STATUS_PATH_CHARS: usize = 60;

/// A long path with its first folders replaced by "…", keeping the file and the folders
/// nearest to it, which say the most: `…/editor/assist.rs`.
fn shorten_path(path: &str, max: usize) -> String {
    if path.chars().count() <= max {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    let mut kept = parts.last().copied().unwrap_or(path).to_string();
    for part in parts.iter().rev().skip(1) {
        if kept.chars().count() + part.chars().count() + 3 > max {
            break;
        }
        kept = format!("{part}/{kept}");
    }
    format!("…/{kept}")
}

/// The `null` command: each path made absolute (created when missing, as editors do),
/// then handed to Null with `launch`, which reads it as `$p`.
fn shell_script(launch: &str) -> String {
    format!(
        "#!/bin/sh\n# Opens files and folders in Null: `null .`, `null src/main.rs`.\n\
         [ $# -eq 0 ] && set -- .\n\
         for p in \"$@\"; do\n\
         \tcase \"$p\" in /*) ;; *) p=\"$PWD/$p\" ;; esac\n\
         \t[ -e \"$p\" ] || : > \"$p\"\n\
         \t{launch}\n\
         done\n"
    )
}

/// Where `path` ends up when `from` (it, or a folder above it) is renamed to `to`.
fn moved_path(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix(from).ok()?;
    // Joining an empty rest would add a trailing slash and turn the file into a "folder".
    Some(if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) })
}

#[derive(Clone)]
enum CloseAction {
    Quit,
    CloseWindow,
    CloseTabs(Vec<Entity<Editor>>),
    /// Open another folder as the project: every tab closes first.
    SwitchProject(PathBuf),
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (chrome, fading) = self.chrome.value(FADE_IN, FADE_OUT);
        let (sidebar, sliding) = self.sidebar.value(SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        let (searching, switching) = self.sidebar_search.value(SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        let (terminal_shown, terminal_sliding) = self.terminal_open.value(TERMINAL_SLIDE, TERMINAL_SLIDE);
        if fading || sliding || switching || terminal_sliding {
            window.request_animation_frame();
        }
        let opacity = DIMMED + (1. - DIMMED) * chrome;
        let full_width = SIDEBAR_WIDTH + (SEARCH_SIDEBAR_WIDTH - SIDEBAR_WIDTH) * searching;
        let sidebar_width = full_width * sidebar;
        let switch = self.render_sidebar_switch(cx);
        let sidebar_content = if self.sidebar_search.on {
            div().flex_1().min_h_0().pt(px(6.)).child(self.project_search.clone())
        } else {
            div().flex_1().min_h_0().child(self.tree.clone())
        };

        let root = self.tree.read(cx).root().to_path_buf();
        let lsp_status = self.language_status(cx);
        let (status_items, problems): (Vec<String>, (usize, usize)) = match self.active_editor().map(|e| e.read(cx)) {
            Some(editor) => {
                let (line, col) = editor.caret_point();
                let path = editor.path().map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string());
                let problems = editor.problems(cx);
                let count = |s| problems.iter().filter(|p| p.severity == s).count();
                (
                    vec![
                        path.map(|p| shorten_path(&p, STATUS_PATH_CHARS)).unwrap_or_else(|| "Untitled".into()),
                        match editor.extra.len() {
                            0 => format!("Ln {}, Col {}", line + 1, col + 1),
                            n => format!("{} cursors · Esc for one", n + 1),
                        },
                        editor.language_name().into(),
                    ],
                    (count(lsp_types::DiagnosticSeverity::ERROR), count(lsp_types::DiagnosticSeverity::WARNING)),
                )
            }
            None => {
                let home = std::env::var_os("HOME").map(PathBuf::from);
                let shown = match home.as_ref().and_then(|h| root.strip_prefix(h).ok()) {
                    Some(rest) => Path::new("~").join(rest).display().to_string(),
                    None => root.display().to_string(),
                };
                (vec![shorten_path(&shown, STATUS_PATH_CHARS)], (0, 0))
            }
        };
        let split = self.is_split();
        let tabs_left = self.render_tabs(0, cx);
        let tabs_right = split.then(|| self.render_tabs(1, cx));
        let terminal_panel = self.render_terminal_panel(TERMINAL_HEIGHT * terminal_shown, cx);
        // One editor, or two side by side, each side making its tab current when clicked into.
        let pane = |this: &Self, side: usize, cx: &mut Context<Self>| -> AnyElement {
            let content = match this.shown_editor(side) {
                Some(editor) => editor.clone().into_any_element(),
                None => this.render_empty(cx),
            };
            let theme = cx.global::<Theme>().clone();
            div()
                .id(("pane", side))
                .relative()
                .min_w_0()
                .h_full()
                .capture_any_mouse_down(
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| this.focus_side(side, window, cx)),
                )
                .child(content)
                // A tab dropped on a side moves there.
                .drag_over::<DraggedTab>(move |style, _, _, _| style.bg(theme.accent_soft.opacity(0.5)))
                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                    this.drop_tab(&dragged.editor, None, side, window, cx)
                }))
                .into_any_element()
        };
        let theme_now = cx.global::<Theme>().clone();
        let body = if split {
            let left = div().w(relative(self.split_ratio)).h_full().flex_none().child(pane(self, 0, cx));
            let right = div().flex_1().min_w_0().h_full().child(pane(self, 1, cx));
            // The line between the sides: drag it to resize them.
            let divider = div()
                .id("divider")
                .w(px(7.))
                .mx(px(-3.))
                .h_full()
                .flex_none()
                .flex()
                .justify_center()
                .cursor_col_resize()
                .group("divider")
                .on_drag(DraggedDivider, |_, _, _, cx| cx.new(|_| gpui::EmptyView))
                .child(
                    div()
                        .w(px(1.))
                        .h_full()
                        .bg(theme_now.hairline)
                        .group_hover("divider", |s| s.bg(theme_now.line_strong)),
                );
            div()
                .size_full()
                .flex()
                .on_drag_move::<DraggedDivider>(cx.listener(|this, event: &DragMoveEvent<DraggedDivider>, _, cx| {
                    let x = f32::from(event.event.position.x - event.bounds.left());
                    this.split_ratio = (x / f32::from(event.bounds.size.width)).clamp(0.2, 0.8);
                    this.schedule_session_save(cx);
                    cx.notify();
                }))
                .child(left)
                .child(divider)
                .child(right)
        } else {
            // While a tab is dragged, the right half offers to open it there.
            // (Unsplit, the only thing Null drags is a tab.)
            let dragging_tab = cx.has_active_drag() && self.tabs.len() > 1;
            div().relative().size_full().child(div().size_full().child(pane(self, 0, cx))).when(dragging_tab, |body| {
                body.child(
                    div()
                        .id("drop-right")
                        .absolute()
                        .top_0()
                        .right_0()
                        .w(relative(0.5))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(ui::T_MD))
                        .text_color(theme_now.muted)
                        .border_l_1()
                        .border_color(theme_now.hairline)
                        .drag_over::<DraggedTab>({
                            let tint = theme_now.accent_soft;
                            move |style, _, _, _| style.bg(tint)
                        })
                        .child("Open on the right")
                        .on_drop(cx.listener(|this, dragged: &DraggedTab, window, cx| {
                            this.drop_tab(&dragged.editor, None, 1, window, cx)
                        })),
                )
            })
        };
        // One floating layer at a time: the palette, an AI answer, or the key prompt.
        // The welcome covers the whole window; the rest float over the work.
        let welcome = self.welcome.as_ref().map(|(w, _)| w.clone());
        let overlay: Option<AnyElement> = if let Some((palette, _)) = &self.palette {
            Some(palette.clone().into_any_element())
        } else if let Some((panel, _)) = &self.settings_panel {
            Some(panel.clone().into_any_element())
        } else {
            self.key_prompt.as_ref().map(|(prompt, _)| prompt.clone().into_any_element())
        };
        let ai_provider = cx.global::<Settings>().ai.active();
        // How the file is written, when it's not the usual: its indentation, Windows line endings.
        let default_indent = cx.global::<Settings>().default_indent();
        let (indent_label, crlf) = match self.active_editor().map(|e| e.read(cx).style.clone()) {
            Some(style) => (
                (style.indent != default_indent).then(|| style.indent.label()),
                style.line_ending == crate::file_style::LineEnding::Crlf,
            ),
            None => (None, false),
        };
        let theme = cx.global::<Theme>();

        let titlebar = div()
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(TITLEBAR_INSET))
            .pr(px(12.))
            .border_b_1()
            .border_color(theme.hairline)
            .bg(theme.surface)
            .opacity(opacity)
            .window_control_area(WindowControlArea::Drag)
            .map(|bar| match tabs_right {
                // Each side's tabs over it: the left ones end where the right side begins.
                Some(right) => {
                    let main = f32::from(window.viewport_size().width) - sidebar_width;
                    let left_width = (sidebar_width + main * self.split_ratio - TITLEBAR_INSET).max(80.);
                    bar.child(div().flex_none().w(px(left_width)).pr(px(8.)).child(tabs_left))
                        .child(div().flex_1().min_w_0().pl(px(8.)).child(right))
                }
                None => bar.child(tabs_left),
            });

        let sidebar_panel = div()
            .flex_none()
            .w(px(sidebar_width))
            .h_full()
            .overflow_hidden()
            .bg(theme.surface)
            .when(sidebar_width > 0.5, |panel| panel.border_r_1().border_color(theme.hairline))
            .opacity(opacity)
            .child(div().w(px(full_width)).h_full().flex().flex_col().child(switch).child(sidebar_content));

        let mut items = status_items.into_iter();
        let status =
            div()
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(16.))
                .px(px(16.))
                .border_t_1()
                .border_color(theme.hairline)
                .bg(theme.surface)
                .text_size(px(12.))
                .text_color(theme.muted)
                .opacity(opacity)
                .children(self.branch.clone().map(|branch| {
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(5.))
                        .child(svg().path("icons/branch.svg").size(px(13.)).text_color(theme.muted))
                        .child(branch)
                }))
                .child(div().flex_1().min_w_0().truncate().children(items.next()))
                .children(lsp_status)
                .children(indent_label.map(|label| {
                    div()
                        .id("status-indent")
                        .flex_none()
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme.foreground))
                        .child(label)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.show_commands_for("indent with", window, cx)
                        }))
                }))
                .when(crlf, |bar| {
                    bar.child(
                        div()
                            .id("status-crlf")
                            .flex_none()
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme.foreground))
                            .child("CRLF")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.show_commands_for("line endings", window, cx)
                            })),
                    )
                })
                .children(self.ai_task.as_ref().map(|run| {
                    // One line for a task: what it's at, then how many files to review.
                    let text: String = match &run.state {
                        TaskState::Starting => "Starting the task…".into(),
                        TaskState::Running(Some(file)) => {
                            let name =
                                Path::new(file).file_name().map_or(file.clone(), |n| n.to_string_lossy().into_owned());
                            format!("Working on {name}…")
                        }
                        TaskState::Running(None) => format!("{}…", run.title.trim()),
                        TaskState::Review(changes) if changes.len() == 1 => "Review 1 file".into(),
                        TaskState::Review(changes) => format!("Review {} files", changes.len()),
                    };
                    let reviewing = matches!(run.state, TaskState::Review(_));
                    div()
                        .id("ai-task")
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .whitespace_nowrap()
                        .max_w(px(320.))
                        .text_color(theme.foreground)
                        .child(div().flex_none().text_color(theme.caret).child("✦"))
                        .child(div().min_w_0().truncate().child(text))
                        // While it works, only "Stop" stops it; once done, the line opens the review.
                        .when(!reviewing, |d| {
                            d.child(
                                div()
                                    .id("stop-ai-task")
                                    .flex_none()
                                    .cursor_pointer()
                                    .text_color(theme.muted)
                                    .hover(|s| s.text_color(theme.foreground))
                                    .child("Stop")
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.stop_ai_task(&StopAiTask, window, cx)
                                    })),
                            )
                        })
                        .when(reviewing, |d| {
                            d.cursor_pointer().on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.review_ai_task(&ReviewAiTask, window, cx)
                            }))
                        })
                }))
                .when(ai_provider != ProviderId::Off, |bar| {
                    bar.child(
                        div()
                            .id("ai-status")
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(6.))
                            .whitespace_nowrap()
                            .cursor_pointer()
                            .text_color(theme.muted)
                            .hover(|s| s.text_color(theme.foreground))
                            .child(div().size(px(6.)).rounded_full().bg(theme.caret.opacity(0.6)))
                            .child(format!("AI · {}", ai_provider.label()))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.open_settings_at(Some(Section::Ai), window, cx)
                            })),
                    )
                })
                .when(problems != (0, 0), |bar| {
                    let (errors, warnings) = problems;
                    let plural =
                        |n: usize, word: &str| if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") };
                    bar.child(
                        div()
                            .id("problems")
                            .flex()
                            .flex_none()
                            .whitespace_nowrap()
                            .gap(px(10.))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.show_problems(&ShowProblems, window, cx)
                            }))
                            .when(errors > 0, |d| d.child(div().text_color(theme.error).child(plural(errors, "error"))))
                            .when(warnings > 0, |d| {
                                d.child(div().text_color(theme.warning).child(plural(warnings, "warning")))
                            }),
                    )
                })
                .children(items.map(|item| div().flex_none().whitespace_nowrap().child(item)));

        div()
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(cx.global::<Fonts>().ui.clone())
            .relative()
            .on_action(cx.listener(Self::toggle_palette))
            .on_action(cx.listener(Self::open_settings_file))
            .on_action(cx.listener(Self::show_commands))
            .on_action(cx.listener(Self::toggle_ai))
            .on_action(cx.listener(Self::show_problems))
            .on_action(cx.listener(Self::replace_in_project))
            .on_action(cx.listener(Self::new_ai_task))
            .on_action(cx.listener(|this, _: &ShowWelcome, window, cx| this.show_welcome(window, cx)))
            .on_action(cx.listener(Self::install_shell_command))
            .on_action(cx.listener(Self::review_ai_task))
            .on_action(cx.listener(Self::stop_ai_task))
            .on_action(cx.listener(Self::keep_all_task_changes))
            .on_action(cx.listener(Self::undo_all_task_changes))
            .on_action(cx.listener(|this, _: &MoveTabRight, window, cx| this.move_tab_to(1, window, cx)))
            .on_action(cx.listener(|this, _: &MoveTabLeft, window, cx| this.move_tab_to(0, window, cx)))
            .on_action(cx.listener(Self::go_to_symbol))
            .on_action(cx.listener(Self::go_to_symbol_in_project))
            .on_action(cx.listener(|_, _: &ToggleFormatOnSave, _, cx| {
                settings::update(cx, |s| s.format_on_save = !s.format_on_save)
            }))
            .on_action(cx.listener(Self::ask_ai))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_fade_while_typing))
            .on_action(cx.listener(Self::toggle_word_wrap))
            .on_action(cx.listener(Self::toggle_autocomplete))
            .on_action(cx.listener(Self::toggle_terminal))
            .on_action(cx.listener(Self::go_to_line))
            .on_action(cx.listener(Self::new_untitled))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::save_active))
            .on_action(cx.listener(Self::save_all))
            .on_action(cx.listener(Self::reopen_closed_tab))
            .on_action(cx.listener(Self::close_all_tabs))
            .on_action(cx.listener(Self::close_other_tabs))
            .on_action(cx.listener(Self::use_nvidia))
            .on_action(cx.listener(Self::use_ollama))
            .on_action(cx.listener(Self::use_openai_compatible))
            .on_action(cx.listener(Self::use_claude_api))
            .on_action(cx.listener(Self::use_claude_code))
            .on_action(cx.listener(Self::use_codex))
            .on_action(cx.listener(Self::turn_off_ai))
            .on_action(cx.listener(Self::set_api_key))
            .on_action(cx.listener(Self::increase_font_size))
            .on_action(cx.listener(Self::decrease_font_size))
            .on_action(cx.listener(Self::reset_font_size))
            .on_action(cx.listener(Self::use_oled_theme))
            .on_action(cx.listener(Self::use_graphite_theme))
            .on_action(cx.listener(Self::use_paper_theme))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::search_project))
            .on_action(cx.listener(Self::show_files))
            .on_action(cx.listener(Self::quit))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(titlebar)
            .child(
                div().flex_1().min_h_0().flex().child(sidebar_panel).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().flex_1().min_h_0().child(body))
                        .children(terminal_panel),
                ),
            )
            .child(status)
            .when_some(overlay, |root, layer| {
                root.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .justify_center()
                        .items_start()
                        .px(px(16.))
                        .pt(px(88.))
                        .bg(theme.scrim)
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                // The welcome is closed by its own button, not by clicking around it.
                                if this.welcome.is_some() {
                                    return;
                                }
                                this.key_prompt = None;
                                this.close_palette(window, cx);
                            }),
                        )
                        .child(layer),
                )
            })
            .children(welcome.map(|welcome| {
                div().absolute().top_0().left_0().size_full().occlude().child(welcome).into_any_element()
            }))
            .children(self.notice.as_ref().map(|(message, _)| {
                div().absolute().bottom(px(44.)).left_0().w_full().flex().justify_center().child(
                    div()
                        .max_w(px(560.))
                        .px(px(14.))
                        .py(px(8.))
                        .rounded(px(ui::R_POPOVER))
                        .bg(theme.raised)
                        .border_1()
                        .border_color(theme.hairline)
                        .shadow_lg()
                        .text_size(px(ui::T_MD))
                        .text_color(theme.foreground)
                        .child(message.clone()),
                )
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn two_sides_open_move_and_close_back_to_one(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-split-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.join(name), name).unwrap();
        }
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let names = |w: &Workspace, cx: &App, side: usize| -> Vec<String> {
            w.side_tabs(side).iter().map(|&i| w.tabs[i].editor.read(cx).file_name()).collect()
        };
        workspace.update_in(cx, |w, window, cx| {
            for name in ["a.txt", "b.txt"] {
                w.open_file(dir.join(name), window, cx);
            }
            // b moves right: a stays on the left.
            w.move_tab_to(1, window, cx);
            assert!(w.is_split());
            assert_eq!((names(w, cx, 0), names(w, cx, 1)), (vec!["a.txt".into()], vec!["b.txt".into()]));
            // Files open on the side being worked in.
            w.open_file(dir.join("c.txt"), window, cx);
            assert_eq!(names(w, cx, 1), vec!["b.txt".to_string(), "c.txt".to_string()]);
            assert_eq!(w.shown_editor(1).map(|e| e.read(cx).file_name()), Some("c.txt".into()));
            // Next tab stays on its side.
            w.next_tab(&NextTab, window, cx);
            assert_eq!(w.active_editor().map(|e| e.read(cx).file_name()), Some("b.txt".into()));
            // Dragging c before b reorders the right side; dropping b on the left moves it there.
            let (b, c) = (w.side_tabs(1)[0], w.side_tabs(1)[1]);
            let c_editor = w.tabs[c].editor.clone();
            w.drop_tab(&c_editor, Some(b), 1, window, cx);
            assert_eq!(names(w, cx, 1), vec!["c.txt".to_string(), "b.txt".to_string()]);
            let b_editor = w.tabs[w.side_tabs(1)[1]].editor.clone();
            w.drop_tab(&b_editor, None, 0, window, cx);
            assert_eq!(names(w, cx, 0), vec!["a.txt".to_string(), "b.txt".to_string()]);
            assert_eq!(names(w, cx, 1), vec!["c.txt".to_string()]);
            assert_eq!(w.shown_editor(1).map(|e| e.read(cx).file_name()), Some("c.txt".into()));
            // Back to b on the right for what follows.
            w.drop_tab(&b_editor, None, 1, window, cx);
            let c_editor = w.tabs[w.side_tabs(1)[0]].editor.clone();
            w.drop_tab(&c_editor, None, 1, window, cx);
            w.activate(w.side_tabs(1)[0], window, cx);
            // Closing the left's only tab: the right becomes the only side.
            let a = w.side_tabs(0)[0];
            w.remove_tab(a, window, cx);
            assert!(!w.is_split());
            assert_eq!(names(w, cx, 0), vec!["b.txt".to_string(), "c.txt".to_string()]);
            assert!(w.shown_editor(0).is_some());
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_shell_command_is_valid_and_hands_over_absolute_paths() {
        let dir = std::env::temp_dir().join(format!("null-cmd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log");
        // A stand-in for Null that writes down what it was given.
        let script = shell_script(&format!("echo \"$p\" >> \"{}\"", log.display()));
        let command = dir.join("null");
        std::fs::write(&command, script).unwrap();
        let run = std::process::Command::new("sh").arg(&command).args(["new.txt", "."]).current_dir(&dir).status();
        assert!(run.unwrap().success());
        let given = std::fs::read_to_string(&log).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        assert!(given.lines().next().unwrap().ends_with("/new.txt"), "{given}");
        assert!(dir.join("new.txt").exists());
        assert_eq!(given.lines().count(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn long_paths_keep_their_end() {
        assert_eq!(shorten_path("src/main.rs", 60), "src/main.rs");
        assert_eq!(shorten_path("a/very/deep/tree/of/folders/editor/assist.rs", 24), "…/editor/assist.rs");
    }

    #[test]
    fn renamed_paths_never_gain_a_trailing_slash() {
        let (from, to) = (Path::new("/p/old.py"), Path::new("/p/test.py"));
        assert_eq!(moved_path(from, from, to).unwrap(), PathBuf::from("/p/test.py"));
        assert!(!moved_path(from, from, to).unwrap().to_string_lossy().ends_with('/'));
        let (dir, new_dir) = (Path::new("/p/src"), Path::new("/p/lib"));
        assert_eq!(moved_path(Path::new("/p/src/a/b.rs"), dir, new_dir).unwrap(), PathBuf::from("/p/lib/a/b.rs"));
        assert_eq!(moved_path(Path::new("/p/other.rs"), dir, new_dir), None);
    }
}
