use crate::ai::ProviderId;
use crate::editor::{Editor, EditorEvent, GoToDefinition, Redo, Save, SelectAll, ShowInfo, Undo};
use crate::file_tree::{FileTree, FileTreeEvent};
use crate::find_bar::{DeployFind, DeployReplace};
use crate::fonts::CodeFont;
use crate::fonts::Fonts;
use crate::git;
use crate::key_prompt::{KeyPrompt, KeyPromptEvent};
use crate::lsp_store::{LspStore, Readiness};
use crate::menus::{self, Quit, ToggleFadeWhileTyping, ToggleWordWrap};
use crate::palette::{Category, Command, Palette, PaletteEvent, PaletteKind, PaletteOptions, format_keys};
use crate::project_search::{ProjectSearch, ProjectSearchEvent};
use crate::settings::AutoSave;
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
        OpenOnOtherSide,
        NewAiTask,
        ReviewChanges,
        CommitAll,
        UndoLastCommit,
        SetChangesAside,
        BringBackChanges,
        OpenInBrowser,
        FindTodos,
        PushBranch,
        PullBranch,
        FileHistory,
        CompareWithSaved,
        CopyLineLink,
        OpenLineOnWeb,
        FetchBranch,
        CopyFilePath,
        CopyRelativeFilePath,
        CopyAsCodeBlock,
        RevealFile,
        RenameFile,
        TrashFile,
        CompareWithClipboard,
        CompareWithFile,
        RevertAllChanges,
        SwitchBranch,
        GoBack,
        GoForward,
        GoToLastEdit,
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
        AutoSaveOff,
        ToggleLineBlame,
        ToggleSpellCheck,
        ToggleInlayHints,
        ToggleFocusMode,
        RunTask,
        RunTestAtCursor,
        RunTestsInFile,
        OpenPreviewToTheSide,
        OpenRecent,
        NewWindow,
        StartDebugging,
        StopDebugging,
        StepOver,
        StepInto,
        StepOut,
        PauseDebugging,
        NextProblem,
        ToggleIndentGuides,
        ToggleLineGuide,
        ToggleStickyScroll,
        ToggleSymbolMarks,
        PreviousProblem,
        AutoSaveAfterPause,
        AutoSaveWhenLeaving,
        OpenSettings,
        OpenSettingsFile,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        CompactLineSpacing,
        NormalLineSpacing,
        RelaxedLineSpacing,
        UseNullTheme,
        UseAshTheme,
        UseMidnightTheme,
        UseMossTheme,
        UsePaperTheme,
        UseDuneTheme,
        SearchProject,
        ReplaceInProject,
        ShowFiles,
        ToggleAutocomplete,
        ToggleTerminal,
        NewTerminal,
        NextTerminal,
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
        KeyBinding::new("alt-secondary-enter", ToggleFocusMode, ctx),
        KeyBinding::new("secondary-shift-b", RunTask, ctx),
        KeyBinding::new("alt-secondary-t", RunTestAtCursor, ctx),
        KeyBinding::new("alt-secondary-o", OpenRecent, ctx),
        KeyBinding::new("secondary-shift-n", NewWindow, ctx),
        KeyBinding::new("f5", StartDebugging, ctx),
        KeyBinding::new("shift-f5", StopDebugging, ctx),
        KeyBinding::new("f6", PauseDebugging, ctx),
        KeyBinding::new("f10", StepOver, ctx),
        KeyBinding::new("f11", StepInto, ctx),
        KeyBinding::new("shift-f11", StepOut, ctx),
        KeyBinding::new("f8", NextProblem, ctx),
        KeyBinding::new("shift-f8", PreviousProblem, ctx),
        KeyBinding::new("ctrl-tab", NextTab, ctx),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, ctx),
        KeyBinding::new("secondary-,", OpenSettings, ctx),
        KeyBinding::new("secondary-shift-f", SearchProject, ctx),
        KeyBinding::new("secondary-shift-h", ReplaceInProject, ctx),
        // Arrows rather than ⌘\, which takes several keys on many layouts.
        KeyBinding::new("alt-secondary-i", NewAiTask, ctx),
        KeyBinding::new("ctrl-shift-g", ReviewChanges, ctx),
        KeyBinding::new("ctrl-secondary-right", MoveTabRight, ctx),
        KeyBinding::new("ctrl-alt-secondary-right", OpenOnOtherSide, ctx),
        KeyBinding::new("ctrl-secondary-left", MoveTabLeft, ctx),
        KeyBinding::new("secondary-shift-e", ShowFiles, ctx),
        KeyBinding::new("ctrl-`", ToggleTerminal, ctx),
        KeyBinding::new("ctrl-shift-`", NewTerminal, ctx),
        KeyBinding::new("ctrl-~", NewTerminal, ctx),
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
        KeyBinding::new("secondary-shift-backspace", GoToLastEdit, ctx),
    ];
    if cfg!(target_os = "macos") {
        keys.extend([
            KeyBinding::new("cmd-shift-]", NextTab, ctx),
            KeyBinding::new("cmd-shift-[", PreviousTab, ctx),
            KeyBinding::new("ctrl--", GoBack, ctx),
            KeyBinding::new("ctrl-r", OpenRecent, ctx),
            KeyBinding::new("ctrl-shift--", GoForward, ctx),
            KeyBinding::new("ctrl-_", GoForward, ctx),
        ]);
    } else {
        keys.extend([KeyBinding::new("alt-left", GoBack, ctx), KeyBinding::new("alt-right", GoForward, ctx)]);
    }
    cx.bind_keys(keys);
}

/// What a tab's right-click menu offers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TabMenuItem {
    Close,
    CloseOthers,
    CloseToTheRight,
    CopyPath,
    CopyRelativePath,
    Reveal,
    OtherSide,
    History,
    CopyLink,
}

impl TabMenuItem {
    fn label(self) -> &'static str {
        match self {
            TabMenuItem::Close => "Close",
            TabMenuItem::CloseOthers => "Close Others",
            TabMenuItem::CloseToTheRight => "Close Tabs to the Right",
            TabMenuItem::CopyPath => "Copy Path",
            TabMenuItem::CopyRelativePath => "Copy Relative Path",
            TabMenuItem::Reveal => crate::file_tree::REVEAL_LABEL,
            TabMenuItem::OtherSide => "Open on the Other Side Too",
            TabMenuItem::History => "Show History",
            TabMenuItem::CopyLink => "Copy Link to Line",
        }
    }

    /// Items that start a group get a line above them.
    fn starts_group(self) -> bool {
        matches!(self, TabMenuItem::CopyPath | TabMenuItem::OtherSide)
    }
}

/// A tab's right-click menu: the tab, and where the click was.
struct TabMenu {
    editor: Entity<Editor>,
    position: Point<Pixels>,
}

/// The program `cargo build` makes for a project: its first `[[bin]]`, or the package
/// named in Cargo.toml, in target/debug.
fn cargo_program(root: &Path) -> Option<PathBuf> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut section = String::new();
    let (mut package, mut bin) = (None, None);
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        if key.trim() != "name" {
            continue;
        }
        let value = value.trim().trim_matches('"').to_string();
        match section.as_str() {
            "[[bin]]" if bin.is_none() => bin = Some(value),
            "[package]" => package = Some(value),
            _ => {}
        }
    }
    let name = bin.or(package)?;
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name };
    Some(root.join("target").join("debug").join(exe))
}

/// A place visited, to come back to with Back and Forward.
#[derive(Clone, Debug, PartialEq)]
struct Place {
    path: PathBuf,
    /// Line and column of the caret.
    point: (usize, usize),
}

/// How many places Back remembers.
const MAX_PLACES: usize = 100;

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
/// How many characters wide Focus mode's column of text is.
const FOCUS_COLUMNS: f32 = 100.;
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
    /// The message of a commit taken back, waiting in Commit's field for the next one.
    commit_draft: Option<String>,
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
    /// What git says changed since the last commit, and the task reading it.
    git_status: Vec<(PathBuf, git::FileStatus)>,
    git_status_task: Option<Task<()>>,
    /// The list of changes is open: opening a file from it shows its changes.
    git_listing: bool,
    /// The history a list shows (⌥ picking one compares the file with it), and its file.
    history: Option<(PathBuf, Vec<Past>)>,
    /// A tab's right-click menu, while open.
    tab_menu: Option<TabMenu>,
    /// Only the code: no sidebar, tabs, status bar or terminal, the text centered.
    focus_mode: bool,
    /// Places to go back and forward to, most recent last.
    back: Vec<Place>,
    forward: Vec<Place>,
    /// Going back or forward: the moves it makes aren't places to remember.
    navigating: bool,
    /// The file Compare with File… was asked from, while a file to compare it with is picked.
    compare_from: Option<Entity<Editor>>,
    /// Where the last edit was made, in any file: Go to Last Edit goes back there.
    last_edit: Option<Place>,
    /// Commands run from ⌘⇧B, the last first.
    recent_runs: Vec<String>,
    /// The debugger, and its panel (shown while debugging, and after, until closed).
    debugger: Entity<crate::debugger::Debugger>,
    debug_panel_open: bool,
    debug_output_scroll: gpui::ScrollHandle,
    /// While paused, the panel shows the variables and the calls; this shows the output instead.
    debug_show_output: bool,
    /// The paused call's variables the code shows, with values worth showing.
    debug_locals: Vec<(String, String)>,
    /// The program to debug, when the project doesn't say (no Cargo.toml): asked once.
    debug_program: Option<PathBuf>,
    /// The projects for the list about to open.
    pending_projects: Vec<PathBuf>,
    /// The tasks for the list about to open.
    pending_tasks: Vec<crate::tasks::ProjectTask>,
    /// The branches for the branch list about to open.
    pending_branches: Vec<git::Branch>,
    /// The AI task running or waiting for review, if any.
    ai_task: Option<AiTaskRun>,
    /// Commands the next palette list shows after its places (a task's "Keep all").
    pending_commands: Vec<crate::palette::Command>,
    /// For a file open on both sides: each copy's buffer revision last passed to the other.
    twin_seen: std::collections::HashMap<gpui::EntityId, u64>,
    /// Quit was asked and this window's unsaved changes were answered for.
    pub quitting: bool,
    /// Writing unsaved work to its backup, once typing pauses.
    backup_task: Option<Task<()>>,
    /// Saves waiting for a pause in typing, by editor.
    auto_saves: std::collections::HashMap<gpui::EntityId, Task<()>>,
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
    /// The shells open in the terminal panel, and the one shown.
    terminals: Vec<(Entity<TerminalView>, Subscription)>,
    active_terminal: usize,
    terminal_open: Transition,
    /// The current git branch of the project, if it's a repository.
    branch: Option<String>,
    /// Commits to push and to pull, against the branch's upstream as last fetched.
    sync: Option<(usize, usize)>,
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
        let tree = cx.new(|cx| {
            let mut tree = FileTree::new(root, cx);
            // Renaming a file can update the code that names it: the workspace sees to it.
            tree.ask_before_renaming = true;
            tree
        });
        let subscriptions = vec![
            cx.subscribe_in(&tree, window, |this, _, event, window, cx| match event {
                FileTreeEvent::Open(path) | FileTreeEvent::Created(path) => this.open_file(path.clone(), window, cx),
                FileTreeEvent::Preview(path) => {
                    this.open_file(path.clone(), window, cx);
                    window.focus(&this.tree.focus_handle(cx));
                }
                FileTreeEvent::Renamed { from, to } => this.paths_renamed(from, to, cx),
                FileTreeEvent::RenameRequested { from, to } => this.rename_file(from.clone(), to.clone(), cx),
                FileTreeEvent::Trashed(path) => this.path_trashed(path, window, cx),
                FileTreeEvent::OpenTerminal(dir) => this.open_terminal_in(dir.clone(), window, cx),
                FileTreeEvent::DiscardChanges(path, status) => this.discard_changes(path.clone(), *status, window, cx),
                FileTreeEvent::Notice(message) => this.show_notice(message.clone(), cx),
            }),
            cx.observe_global::<Settings>(|this, cx| this.apply_settings(cx)),
            cx.observe(&lsp, |_, _, cx| cx.notify()),
            cx.subscribe(&lsp, |this, _, event, cx| match event {
                crate::lsp_store::LspEvent::Installed(result) => {
                    let message = match result {
                        Ok(name) => format!("{name} is installed"),
                        Err(reason) => reason.clone(),
                    };
                    this.show_notice(message, cx);
                }
                crate::lsp_store::LspEvent::ApplyEdit(edit) => this.apply_fix_edit(edit.clone(), cx),
                crate::lsp_store::LspEvent::DiagnosticsChanged => {}
            }),
            cx.subscribe_in(&project_search, window, |this, _, event, window, cx| match event {
                ProjectSearchEvent::Open { path, line, columns, query, keep_focus } => {
                    let (line, columns, query) = (*line, columns.clone(), query.clone());
                    // It becomes the latest search: ⌘G and ⌘F in any file go on with it.
                    crate::find_bar::remember_search(&query.text, cx);
                    if this.active_editor().and_then(|e| e.read(cx).path().map(Path::to_path_buf)).as_ref()
                        == Some(path)
                    {
                        let here = this.here(cx);
                        this.remember_place(here);
                    }
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
        let debugger = cx.new(|_| crate::debugger::Debugger::default());
        let debugger_events =
            cx.subscribe_in(&debugger, window, |this, _, event, window, cx| this.debugger_event(event, window, cx));
        let debugger_changes = cx.observe(&debugger, |this, _, cx| {
            // New output: keep the end of it in sight.
            this.debug_output_scroll.scroll_to_bottom();
            cx.notify();
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
            commit_draft: None,
            settings_panel: None,
            welcome: None,
            ready_since: None,
            branch: None,
            sync: None,
            branch_task: None,
            terminals: Vec::new(),
            active_terminal: 0,
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
            twin_seen: Default::default(),
            auto_saves: Default::default(),
            backup_task: None,
            quitting: false,
            view_before_preview: None,
            ai_task: None,
            git_status: Vec::new(),
            git_status_task: None,
            git_listing: false,
            history: None,
            pending_branches: Vec::new(),
            recent_runs: Vec::new(),
            pending_tasks: Vec::new(),
            pending_projects: Vec::new(),
            debugger: debugger.clone(),
            debug_panel_open: false,
            debug_output_scroll: gpui::ScrollHandle::new(),
            debug_show_output: false,
            debug_locals: Vec::new(),
            debug_program: None,
            focus_mode: false,
            tab_menu: None,
            back: Vec::new(),
            forward: Vec::new(),
            navigating: false,
            compare_from: None,
            last_edit: None,
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
            this.write_backups_now(cx);
            this.save_session(cx);
            async {}
        })
        .detach();
        workspace._subscriptions.push(debugger_events);
        workspace._subscriptions.push(debugger_changes);
        workspace._subscriptions.push(cx.observe_window_bounds(window, |this, window, cx| {
            this.window_state = Some(window_state(window));
            this.schedule_session_save(cx);
        }));
        workspace._subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                // Off to another app: files save now when they save by themselves.
                if cx.global::<Settings>().auto_save != AutoSave::Off {
                    this.save_named_tabs(cx);
                }
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
                    breakpoints: editor.breakpoints.clone(),
                    conditions: editor.breakpoint_conditions.clone(),
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
            let side = tab.side.min(1);
            let other_side_copy = self
                .tabs
                .iter()
                .find(|t| t.side != side && t.editor.read(cx).path() == Some(tab.path.as_path()))
                .map(|t| t.editor.clone());
            match other_side_copy {
                Some(source) => self.add_twin(&source, side, window, cx),
                None => self.open_file_on(tab.path.clone(), Some(side), window, cx),
            }
            if let Some(editor) = self.active_editor() {
                editor.update(cx, |editor, cx| {
                    editor.restore_folds(&tab.folds, cx);
                    let lines = editor.buffer.len_lines();
                    editor.breakpoints = tab.breakpoints.iter().copied().filter(|&l| l < lines).collect();
                    editor.breakpoint_conditions =
                        tab.conditions.iter().filter(|(l, _)| editor.breakpoints.contains(l)).cloned().collect();
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
        // Unsaved work from last time, if Null didn't get to close properly.
        self.restore_backups(window, cx);
        // Opening the tabs again isn't somewhere to go back to.
        self.back.clear();
        self.forward.clear();
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
                            if let Ok((text, _)) = crate::encoding::read(path) {
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
            let Some(path) = tab.editor.read(cx).path().map(Path::to_path_buf) else { continue };
            if visible.iter().any(|v| *v == path) {
                tab.editor.update(cx, |editor, cx| editor.reload_from_disk(cx));
            } else if visible.iter().any(|v| path.starts_with(v)) {
                // Its folder changed (deleted, renamed): only whether the file is still there.
                tab.editor.update(cx, |editor, cx| editor.check_missing(cx));
            }
        }
        self.follow_moves(&visible, cx);
        cx.notify();
        if git_changed {
            self.refresh_git(cx);
        } else if !visible.is_empty() {
            self.refresh_git_status(cx);
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
        self.refresh_git_status(cx);
    }

    /// Asks git what changed (off the main thread), for the tree and the status bar.
    fn refresh_git_status(&mut self, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        self.git_status_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            let (changed, sync) =
                cx.background_executor().spawn(async move { (git::status(&root), git::ahead_behind(&root)) }).await;
            this.update(cx, |this, cx| {
                this.tree.update(cx, |tree, cx| tree.set_git_status(changed.clone(), cx));
                this.git_status = changed;
                this.sync = sync;
                cx.notify();
            })
            .ok();
        }));
    }

    // ---------- git ----------

    /// ⌃⇧G: the files changed since the last commit, to open one by one (each shows its
    /// changes, to keep or revert), then commit, push or revert them all.
    fn review_changes(&mut self, _: &ReviewChanges, window: &mut Window, cx: &mut Context<Self>) {
        if self.git_status.is_empty() {
            let message = if self.branch.is_some() {
                "Nothing changed since the last commit."
            } else {
                "This folder isn't a git repository."
            };
            return self.show_notice(message.into(), cx);
        }
        self.with_changed_files(window, cx, |this, locations, window, cx| {
            use crate::palette::{Category, Command};
            let command = |label: &str, action: Box<dyn Action>| Command {
                category: Category::File,
                label: label.to_string().into(),
                action,
                keys: None,
            };
            this.pending_commands = vec![
                command("Commit…", Box::new(CommitAll)),
                command("Undo Last Commit", Box::new(UndoLastCommit)),
                command("Set Changes Aside", Box::new(SetChangesAside)),
                command("Push", Box::new(PushBranch)),
                command("Pull", Box::new(PullBranch)),
                command("Switch Branch…", Box::new(SwitchBranch)),
                command("Revert All Changes…", Box::new(RevertAllChanges)),
            ];
            let title = match &this.branch {
                Some(branch) => format!("Changes on {branch}"),
                None => "Changes".to_string(),
            };
            this.open_locations(title, locations, window, cx);
            this.git_listing = true;
        });
    }

    /// The files changed since the last commit, as rows with the lines added and removed
    /// in each (read off the main thread), handed to `then`.
    fn with_changed_files(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Vec<crate::palette::Location>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let root = self.tree.read(cx).root().to_path_buf();
        let changed = self.git_status.clone();
        cx.spawn_in(window, async move |this, cx| {
            let rows = cx
                .background_executor()
                .spawn(async move {
                    changed
                        .into_iter()
                        .map(|(path, status)| {
                            let (after, encoding) = crate::encoding::read(&path).unwrap_or_default();
                            let before = git::committed_text(&path, encoding).unwrap_or_default();
                            let (added, removed) = crate::ai_task::line_counts(&before, &after);
                            (path, status, added, removed)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                use crate::ai_task::ChangeKind;
                use crate::palette::{Location, LocationKind};
                let locations = rows
                    .into_iter()
                    .map(|(path, status, added, removed)| Location {
                        text: path.strip_prefix(&root).unwrap_or(&path).display().to_string(),
                        path,
                        position: lsp_types::Position::default(),
                        kind: LocationKind::FileChange(
                            match status {
                                git::FileStatus::Added => ChangeKind::Added,
                                git::FileStatus::Deleted => ChangeKind::Deleted,
                                _ => ChangeKind::Changed,
                            },
                            added,
                            removed,
                        ),
                    })
                    .collect();
                then(this, locations, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// From the changes list: the file opens with its changes since the last commit, to
    /// keep or revert one by one. A deleted file is restored.
    fn review_git_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(&(_, status)) = self.git_status.iter().find(|(p, _)| p == path) else { return false };
        if status == git::FileStatus::Deleted {
            let root = self.tree.read(cx).root().to_path_buf();
            let message = match git::revert(&root, path, status) {
                Ok(()) => format!("Restored {}", path.file_name().unwrap_or_default().to_string_lossy()),
                Err(error) => format!("Couldn't restore it: {error}"),
            };
            self.show_notice(message, cx);
            self.refresh_git_status(cx);
            return true;
        }
        let encoding = crate::encoding::read(path).map(|(_, e)| e).unwrap_or_default();
        let before = git::committed_text(path, encoding).unwrap_or_default();
        self.open_file(path.to_path_buf(), window, cx);
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |editor, cx| editor.start_review(before, cx));
        }
        true
    }

    /// Asks for a message, then commits the changes, the files under it with ⇥ left out
    /// (open files saved first).
    fn commit_all(&mut self, _: &CommitAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.git_status.is_empty() {
            return self.show_notice("Nothing to commit.".into(), cx);
        }
        self.with_changed_files(window, cx, |this, locations, window, cx| {
            let branch = this.branch.clone().unwrap_or_default();
            this.open_palette_with(PaletteKind::Commit, Some(branch), locations, window, cx);
            if let (Some(draft), Some((palette, _))) = (&this.commit_draft, &this.palette) {
                palette.update(cx, |palette, cx| palette.set_query(draft, cx));
            }
        });
    }

    /// Takes the last commit back (not one already pushed): its changes stay, and its
    /// message waits in Commit's field, to commit them again with more or reworded.
    fn undo_last_commit(&mut self, _: &UndoLastCommit, _: &mut Window, cx: &mut Context<Self>) {
        if self.branch.is_none() {
            return self.show_notice("This folder isn't a git repository.".into(), cx);
        }
        let root = self.tree.read(cx).root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { git::undo_last_commit(&root) }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(message) => {
                        let subject = message.lines().next().unwrap_or_default().to_string();
                        this.commit_draft = Some(message);
                        this.show_notice(format!("Took back “{subject}”: its changes wait for the next Commit."), cx);
                    }
                    Err(error) => this.show_notice(error, cx),
                }
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Sets the changes since the last commit aside (open files saved first): the files go
    /// back to the last commit, to work on something else, until Bring Back Changes.
    fn set_changes_aside(&mut self, _: &SetChangesAside, _: &mut Window, cx: &mut Context<Self>) {
        if self.branch.is_none() {
            return self.show_notice("This folder isn't a git repository.".into(), cx);
        }
        self.save_named_tabs(cx);
        let root = self.tree.read(cx).root().to_path_buf();
        let files = match self.git_status.len() {
            1 => "1 file's changes".to_string(),
            n => format!("{n} files' changes"),
        };
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { git::set_aside(&root) }).await;
            this.update(cx, |this, cx| {
                let notice = match result {
                    Ok(()) => format!("Set {files} aside: Bring Back Changes returns them."),
                    Err(error) => error,
                };
                this.show_notice(notice, cx);
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Returns the changes set aside last. Conflicts open at the first, where ⌘. resolves each.
    fn bring_back_changes(&mut self, _: &BringBackChanges, window: &mut Window, cx: &mut Context<Self>) {
        if self.branch.is_none() {
            return self.show_notice("This folder isn't a git repository.".into(), cx);
        }
        let root = self.tree.read(cx).root().to_path_buf();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { git::bring_back(&root) }).await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(conflicts) if conflicts.is_empty() => this.show_notice("Brought the changes back.".into(), cx),
                    Ok(conflicts) => {
                        let n = conflicts.len();
                        let files = if n == 1 { "1 file".to_string() } else { format!("{n} files") };
                        this.show_notice(format!("Brought back, with conflicts in {files}: ⌘. resolves each."), cx);
                        if let Some(first) = conflicts.into_iter().next() {
                            this.open_file(first, window, cx);
                        }
                    }
                    Err(error) => this.show_notice(error, cx),
                }
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
    }

    // ---------- back and forward ----------

    /// Places a server named, as rows to pick from: each with its line's code, in order.
    fn location_rows(&self, found: Vec<lsp_types::Location>, cx: &App) -> Vec<crate::palette::Location> {
        let mut rows: Vec<crate::palette::Location> = found
            .into_iter()
            .filter_map(|l| {
                let path = crate::lsp::path_for(&l.uri)?;
                let text = self.line_text(&path, l.range.start.line as usize, cx);
                Some(crate::palette::Location {
                    path,
                    position: l.range.start,
                    text: text.trim().to_string(),
                    kind: crate::palette::LocationKind::Reference,
                })
            })
            .collect();
        rows.sort_by(|a, b| {
            (&a.path, a.position.line, a.position.character).cmp(&(&b.path, b.position.line, b.position.character))
        });
        rows
    }

    /// The caret in the current file, as a place to come back to.
    fn here(&self, cx: &App) -> Option<Place> {
        let editor = self.active_editor()?.read(cx);
        Some(Place { path: editor.path()?.to_path_buf(), point: editor.caret_point() })
    }

    /// Remembers a place being left, for Back. A new place ends any way Forward.
    fn remember_place(&mut self, place: Option<Place>) {
        let Some(place) = place.filter(|_| !self.navigating) else { return };
        // Another spot on the same line is the same place.
        if self.back.last().is_some_and(|p| p.path == place.path && p.point.0 == place.point.0) {
            self.back.pop();
        }
        self.back.push(place);
        if self.back.len() > MAX_PLACES {
            self.back.remove(0);
        }
        self.forward.clear();
    }

    /// Opens `path` with `range` selected, remembering where the caret was.
    fn go_to(&mut self, path: PathBuf, range: lsp_types::Range, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.here(cx);
        if here.as_ref().is_some_and(|h| h.path == path) {
            self.remember_place(here);
        }
        self.open_file(path, window, cx);
        if let Some(editor) = self.active_editor() {
            editor.update(cx, |editor, cx| editor.select_lsp_range(range, cx));
        }
    }

    fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(true, window, cx);
    }

    fn go_forward(&mut self, _: &GoForward, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(false, window, cx);
    }

    /// ⇧⌘⌫: back to where the last edit was made, in whichever file (Back returns).
    fn go_to_last_edit(&mut self, _: &GoToLastEdit, window: &mut Window, cx: &mut Context<Self>) {
        let Some(place) = self.last_edit.clone().filter(|p| p.path.is_file()) else {
            return self.show_notice("Nothing edited yet.".into(), cx);
        };
        let here = self.here(cx);
        self.remember_place(here);
        self.navigating = true;
        self.open_file(place.path, window, cx);
        if let Some(editor) = self.active_editor() {
            editor.update(cx, |editor, cx| editor.set_caret_point(place.point, cx));
        }
        self.navigating = false;
    }

    /// Back (or Forward) to the last place, skipping ones that are here already or whose
    /// file is gone.
    fn navigate(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.here(cx);
        loop {
            let next = if back { self.back.pop() } else { self.forward.pop() };
            let Some(place) = next else { return };
            let same = here.as_ref().is_some_and(|h| h.path == place.path && h.point.0 == place.point.0);
            if same || !place.path.is_file() {
                continue;
            }
            if let Some(here) = here {
                if back { self.forward.push(here) } else { self.back.push(here) }
            }
            self.navigating = true;
            self.open_file(place.path, window, cx);
            if let Some(editor) = self.active_editor() {
                editor.update(cx, |editor, cx| editor.set_caret_point(place.point, cx));
            }
            self.navigating = false;
            return;
        }
    }

    /// The branches, to switch to one or start one.
    fn switch_branch(&mut self, _: &SwitchBranch, window: &mut Window, cx: &mut Context<Self>) {
        if self.branch.is_none() {
            return self.show_notice("This folder isn't a git repository.".into(), cx);
        }
        let root = self.tree.read(cx).root().to_path_buf();
        cx.spawn_in(window, async move |this, cx| {
            let branches = cx.background_executor().spawn(async move { git::branches(&root) }).await;
            this.update_in(cx, |this, window, cx| {
                this.pending_branches = branches;
                this.open_palette_with(PaletteKind::Branch, None, Vec::new(), window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Switches to a branch (Ok) or starts one with that name (Err). Open files are saved
    /// first, so git sees every change; it refuses if any would be lost.
    fn change_branch(&mut self, target: Result<git::Branch, String>, window: &mut Window, cx: &mut Context<Self>) {
        self.save_named_tabs(cx);
        let root = self.tree.read(cx).root().to_path_buf();
        let (switch_to, root_now) = (target.clone().ok(), root.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match &target {
                        Ok(branch) => git::switch_branch(&root, branch),
                        Err(name) => git::create_branch(&root, name),
                    }
                    .map(|()| git::current_branch(&root).unwrap_or_default())
                })
                .await;
            // Changes here that branch would overwrite: they can come along instead.
            if let (Err(error), Some(branch)) = (&result, switch_to)
                && error == git::WOULD_LOSE_CHANGES
            {
                this.update_in(cx, |this, window, cx| this.offer_to_carry_changes(branch, root_now, window, cx)).ok();
                return;
            }
            this.update(cx, |this, cx| {
                let notice = match result {
                    Ok(branch) => format!("On {branch}"),
                    Err(error) => error,
                };
                this.show_notice(notice, cx);
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Asks to bring the changes not committed along to `branch`; if so, they're set
    /// aside, the branch switched, and they're put back (conflicts open to resolve).
    fn offer_to_carry_changes(
        &mut self,
        branch: git::Branch,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("Bring your changes along to {}?", branch.name),
            Some("They'd be overwritten there as they are. Null sets them aside, switches, and puts them back."),
            &["Bring Them Along", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let name = branch.name.clone();
            let result =
                cx.background_executor().spawn(async move { git::switch_carrying_changes(&root, &branch) }).await;
            this.update_in(cx, |this, window, cx| {
                let notice = match &result {
                    Ok(conflicts) if conflicts.is_empty() => format!("On {name}, with your changes."),
                    Ok(conflicts) => {
                        let files = if conflicts.len() == 1 {
                            "1 file".to_string()
                        } else {
                            format!("{} files", conflicts.len())
                        };
                        format!("On {name}; your changes conflict in {files}: ⌘. resolves each.")
                    }
                    Err(error) => error.clone(),
                };
                this.show_notice(notice, cx);
                this.refresh_git(cx);
                if let Ok(conflicts) = result
                    && let Some(first) = conflicts.into_iter().next()
                {
                    this.open_file(first, window, cx);
                    if let Some(editor) = this.active_editor().cloned() {
                        editor.update(cx, |e, cx| {
                            e.reload_from_disk(cx);
                            e.go_to_conflict(true, cx);
                        });
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Saves `editor` once typing has paused, unless it's typed in again first.
    fn save_after_pause(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let weak = editor.downgrade();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AutoSave::PAUSE).await;
            if let Some(editor) = weak.upgrade() {
                cx.update(|cx| Self::save_if_named(&editor, cx)).ok();
                this.update(cx, |this, _| this.auto_saves.remove(&editor.entity_id())).ok();
            }
        });
        self.auto_saves.insert(editor.entity_id(), task);
    }

    /// Saves a file with unsaved changes, if it has a name to save under.
    fn save_if_named(editor: &Entity<Editor>, cx: &mut App) {
        let e = editor.read(cx);
        // Not over a file that changed on disk: that waits for a save that asks.
        if e.buffer.is_dirty() && e.path().is_some() && !e.disk_changed {
            editor.update(cx, |editor, cx| editor.save_to_disk(cx));
        }
    }

    /// Saves every open file that has a name.
    fn save_named_tabs(&mut self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            let dirty = tab.editor.read(cx).buffer.is_dirty() && tab.editor.read(cx).path().is_some();
            if dirty {
                tab.editor.update(cx, |editor, cx| editor.save_to_disk(cx));
            }
        }
    }

    fn commit_with(&mut self, message: String, left_out: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.save_named_tabs(cx);
        let root = self.tree.read(cx).root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { git::commit(&root, &message, &left_out) }).await;
            this.update(cx, |this, cx| {
                let notice = match result {
                    Ok(id) => {
                        this.commit_draft = None;
                        format!("Committed {id}.")
                    }
                    Err(error) => format!("Couldn't commit: {error}"),
                };
                this.show_notice(notice, cx);
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The file's past, newest first: the commits that changed it, and the versions Null
    /// wrote over (its local history, git or not). Picking one shows the file against that
    /// version, each change since kept or taken back one by one.
    fn file_history(&mut self, _: &FileHistory, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.active_editor().and_then(|e| e.read(cx).path().map(Path::to_path_buf)) else {
            return self.show_notice("Save the file first: it has no history yet.".into(), cx);
        };
        let file = path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let past = cx
                .background_executor()
                .spawn(async move {
                    let commits = git::file_history(&file, 300).into_iter().map(Past::Commit);
                    let mut past: Vec<Past> =
                        commits.chain(crate::local_history::versions(&file).into_iter().map(Past::Saved)).collect();
                    past.sort_by_key(|p| std::cmp::Reverse(p.time()));
                    past
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if past.is_empty() {
                    return this.show_notice(format!("{name} has no history yet."), cx);
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64);
                let locations = past
                    .iter()
                    .enumerate()
                    .map(|(i, p)| crate::palette::Location {
                        path: path.clone(),
                        // The row's place is its index in the history.
                        position: lsp_types::Position { line: i as u32, character: 0 },
                        text: match p {
                            Past::Commit(c) => {
                                format!("{}\t{} · {} · {}", c.subject, c.author, git::ago(c.time, now), c.short)
                            }
                            Past::Saved(_) => format!("Saved\t{}", git::ago(p.time(), now)),
                        },
                        kind: crate::palette::LocationKind::Commit,
                    })
                    .collect();
                this.open_locations(format!("History of {name}"), locations, window, cx);
                this.history = Some((path, past));
            })
            .ok();
        })
        .detach();
    }

    /// A link to the selected lines (or the caret's) on the repository's site, at the commit
    /// checked out, copied: GitHub, GitLab or Bitbucket.
    /// ⌘I in the commit list: AI reads what changed in `files` and writes the message into
    /// the field as it goes, to edit or commit with ↵.
    fn write_commit_message(&mut self, files: Vec<PathBuf>, cx: &mut Context<Self>) {
        // Weak: once the list is closed, the message stops coming.
        let Some(palette) = self.palette.as_ref().map(|(p, _)| p.downgrade()) else { return };
        let settings = cx.global::<Settings>().ai.clone();
        let root = self.tree.read(cx).root().to_path_buf();
        // Nothing in the field meanwhile: ↵ there must never commit a placeholder.
        self.show_notice("Writing the commit message…".into(), cx);
        cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            let diff = cx
                .background_executor()
                .spawn(async move {
                    let changes: Vec<(String, Option<String>, Option<String>)> = files
                        .iter()
                        .map(|path| {
                            let encoding = crate::encoding::read(path).map(|(_, e)| e).unwrap_or_default();
                            let name = path.strip_prefix(&root).unwrap_or(path).display().to_string();
                            let before = git::committed_text(path, encoding);
                            let after = crate::encoding::read(path).ok().map(|(text, _)| text);
                            (name, before, after)
                        })
                        .collect();
                    changes_as_diff(&changes, COMMIT_DIFF_CHARS)
                })
                .await;
            let prompt = crate::ai::Prompt {
                system: "You write git commit messages. From the changes given, write one: a first line of at most 72 \
                         characters saying what the change does, in the imperative (\"Add…\", \"Fix…\"); then, only if it \
                         helps, a blank line and a few short lines on why. Reply with the message only: no quotes, no \
                         markdown, nothing before or after."
                    .into(),
                user: diff,
                max_tokens: Some(300),
                ..Default::default()
            };
            let mut events = crate::ai::stream(settings, prompt);
            let mut message = String::new();
            while let Some(event) = events.next().await {
                let done = match event {
                    crate::ai::AiEvent::Text(chunk) => {
                        message.push_str(&chunk);
                        false
                    }
                    crate::ai::AiEvent::Done => true,
                    crate::ai::AiEvent::Failed(error) => {
                        this.update(cx, |this, cx| {
                            palette.update(cx, |p, cx| p.set_query("", cx)).ok();
                            this.show_notice(format!("Couldn't write the message: {error}"), cx);
                        })
                        .ok();
                        return;
                    }
                };
                let shown = message.trim().to_string();
                if palette.update(cx, |p, cx| p.set_query(&shown, cx)).is_err() || done {
                    return;
                }
            }
        })
        .detach();
    }

    /// The selected lines (or the caret's) on the clipboard as Markdown, to paste in an
    /// issue or a chat: where they're from, then the code fenced, its common indentation off.
    fn copy_as_code_block(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let root = self.tree.read(cx).root().to_path_buf();
        let e = editor.read(cx);
        let range = e.selection.range();
        let (first, _) = e.buffer.point(range.start);
        let (mut last, col) = e.buffer.point(range.end);
        if col == 0 && last > first {
            last -= 1;
        }
        let lines: Vec<String> = (first..=last).map(|l| e.buffer.line_text(l)).collect();
        let name = match e.path() {
            Some(path) => path.strip_prefix(&root).unwrap_or(path).display().to_string(),
            None => "Untitled".into(),
        };
        let ext = e.path().and_then(|p| p.extension()).and_then(|x| x.to_str()).unwrap_or("").to_string();
        let block = code_block(&name, &ext, first + 1, last + 1, &lines);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(block));
        let what =
            if first == last { format!("line {}", first + 1) } else { format!("lines {}–{}", first + 1, last + 1) };
        self.show_notice(format!("Copied {what} as a code block."), cx);
    }

    fn copy_line_link(&mut self, cx: &mut Context<Self>) {
        self.line_link(false, cx);
    }

    /// The link to the selected lines (or the caret's), copied, or opened in the browser.
    fn line_link(&mut self, open: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        let (path, first, last) = {
            let e = editor.read(cx);
            let Some(path) = e.path().map(Path::to_path_buf) else {
                return self.show_notice("Save the file first.".into(), cx);
            };
            let range = e.selection.range();
            let (first, _) = e.buffer.point(range.start);
            let (mut last, col) = e.buffer.point(range.end);
            // A selection ending at the start of a line doesn't take that line.
            if col == 0 && last > first {
                last -= 1;
            }
            (path, first + 1, last + 1)
        };
        cx.spawn(async move |this, cx| {
            let link = cx.background_executor().spawn(async move { git::line_link(&path, first, last) }).await;
            this.update(cx, |this, cx| {
                let notice = match link {
                    Ok(link) if open => {
                        cx.open_url(&link);
                        return;
                    }
                    Ok(link) => {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(link));
                        if first == last {
                            format!("Copied a link to line {first}.")
                        } else {
                            format!("Copied a link to lines {first}–{last}.")
                        }
                    }
                    Err(error) => format!("Couldn't make a link: {error}"),
                };
                this.show_notice(notice, cx);
            })
            .ok();
        })
        .detach();
    }

    /// What changed since the last save, each change kept or taken back to the file on disk.
    fn compare_with_saved(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        let Some(path) = editor.read(cx).path().map(Path::to_path_buf) else {
            return self.show_notice("This file hasn't been saved yet.".into(), cx);
        };
        let Ok((saved, _)) = crate::encoding::read(&path) else {
            return self.show_notice("Couldn't read the saved file.".into(), cx);
        };
        self.compare_editor(&editor, saved, "Since it was saved", "Nothing changed since it was saved.", cx);
    }

    /// The file against the clipboard: each difference kept, or changed to the clipboard's.
    fn compare_with_clipboard(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        let Some(clipboard) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return self.show_notice("The clipboard has no text.".into(), cx);
        };
        self.compare_editor(&editor, clipboard, "Against the clipboard", "Same as the clipboard.", cx);
    }

    /// Compare with File…: a file picked in the files' list, then the open one against it.
    fn pick_file_to_compare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        self.open_palette_with(PaletteKind::Files, Some("Compare with a file".into()), Vec::new(), window, cx);
        // After opening it: opening the palette closes any before it, which forgets this.
        self.compare_from = Some(editor);
    }

    /// The open file against `path`: each difference kept, or changed to the other file's.
    fn compare_with_file(&mut self, editor: &Entity<Editor>, path: &Path, cx: &mut Context<Self>) {
        let Ok((other, _)) = crate::encoding::read(path) else {
            return self.show_notice("Couldn't read that file.".into(), cx);
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.compare_editor(editor, other, &format!("Against {name}"), &format!("Same as {name}."), cx);
    }

    fn compare_editor(
        &mut self,
        editor: &Entity<Editor>,
        base: String,
        what: &str,
        same: &str,
        cx: &mut Context<Self>,
    ) {
        let changed = editor.update(cx, |editor, cx| {
            editor.start_review(base, cx);
            editor.in_review()
        });
        let notice = if changed { format!("{what}: each change can be kept or taken back.") } else { same.to_string() };
        self.show_notice(notice, cx);
    }

    /// The file against how it was in `commit`: what changed since shows as changes to
    /// keep, or take back to that version, one by one.
    fn compare_with(&mut self, path: &Path, commit: git::FileCommit, window: &mut Window, cx: &mut Context<Self>) {
        let encoding = crate::encoding::read(path).map(|(_, e)| e).unwrap_or_default();
        let Some(before) = git::file_at(path, &commit, encoding) else {
            return self.show_notice(format!("Couldn't read the file as of {}.", commit.short), cx);
        };
        self.open_file(path.to_path_buf(), window, cx);
        let Some(editor) = self.active_editor().cloned() else { return };
        let since = format!("Since {} “{}”", commit.short, commit.subject);
        self.compare_editor(&editor, before, &since, &format!("Unchanged since {}.", commit.short), cx);
    }

    /// The file against a version Null wrote over: what changed since, to keep or take back.
    fn compare_with_version(
        &mut self,
        path: &Path,
        saved: &crate::local_history::Saved,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let encoding = crate::encoding::read(path).map(|(_, e)| e).unwrap_or_default();
        let Some(before) = saved.text(encoding) else {
            return self.show_notice("Couldn't read that version.".into(), cx);
        };
        self.open_file(path.to_path_buf(), window, cx);
        let Some(editor) = self.active_editor().cloned() else { return };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let ago = git::ago(saved.time / 1000, now);
        let (what, same) = (format!("Against the version saved {ago}"), format!("Same as the version saved {ago}."));
        self.compare_editor(&editor, before, &what, &same, cx);
    }

    /// Pulls the branch (open files saved first). Conflicts open at the first one, where
    /// ⌘. resolves each.
    /// Asks the remotes what's new: the counts beside the branch catch up, and say so.
    fn fetch_branch(&mut self, _: &FetchBranch, _: &mut Window, cx: &mut Context<Self>) {
        if self.branch.is_none() {
            return self.show_notice("This folder isn't a git repository.".into(), cx);
        }
        let root = self.tree.read(cx).root().to_path_buf();
        self.show_notice("Fetching…".into(), cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { git::fetch(&root).map(|()| git::ahead_behind(&root)) })
                .await;
            this.update(cx, |this, cx| {
                let commits = |n: usize| if n == 1 { "1 commit".to_string() } else { format!("{n} commits") };
                let notice = match result {
                    Ok(Some((0, 0))) => "Up to date.".to_string(),
                    Ok(Some((ahead, 0))) => format!("{} to push.", commits(ahead)),
                    Ok(Some((0, behind))) => format!("{} to pull.", commits(behind)),
                    Ok(Some((ahead, behind))) => format!("{} to pull, {} to push.", commits(behind), commits(ahead)),
                    Ok(None) => "Fetched. This branch isn't tracking one on a remote.".to_string(),
                    Err(error) => format!("Couldn't fetch: {error}"),
                };
                this.show_notice(notice, cx);
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
    }

    fn pull_branch(&mut self, _: &PullBranch, window: &mut Window, cx: &mut Context<Self>) {
        if self.branch.is_none() {
            return self.show_notice("This folder isn't a git repository.".into(), cx);
        }
        self.save_named_tabs(cx);
        let root = self.tree.read(cx).root().to_path_buf();
        self.show_notice("Pulling…".into(), cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { git::pull(&root) }).await;
            this.update_in(cx, |this, window, cx| {
                let files = |n: usize| if n == 1 { "1 file".to_string() } else { format!("{n} files") };
                let notice = match &result {
                    Ok(git::Pulled::UpToDate) => "Already up to date.".to_string(),
                    Ok(git::Pulled::Commits(1)) => "Pulled 1 commit.".to_string(),
                    Ok(git::Pulled::Commits(n)) => format!("Pulled {n} commits."),
                    Ok(git::Pulled::Conflicts(paths)) => {
                        format!("Pulled, with conflicts in {}: ⌘. resolves each.", files(paths.len()))
                    }
                    Err(error) => format!("Couldn't pull: {error}"),
                };
                this.show_notice(notice, cx);
                this.refresh_git(cx);
                if let Ok(git::Pulled::Conflicts(paths)) = result
                    && let Some(first) = paths.into_iter().next()
                {
                    this.open_file(first, window, cx);
                    if let Some(editor) = this.active_editor().cloned() {
                        // An open copy may not have seen the pull yet.
                        editor.update(cx, |e, cx| {
                            e.reload_from_disk(cx);
                            e.go_to_conflict(true, cx);
                        });
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn push_branch(&mut self, _: &PushBranch, _: &mut Window, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        self.show_notice("Pushing…".into(), cx);
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { git::push(&root) }).await;
            this.update(cx, |this, cx| {
                let notice = match result {
                    Ok(branch) => format!("Pushed {branch}."),
                    Err(error) => format!("Couldn't push: {error}"),
                };
                this.show_notice(notice, cx);
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Every change back to the last commit, after asking (new files go to the Trash).
    /// One file back to its last commit, after asking (a new one goes to the Trash).
    fn discard_changes(&mut self, path: PathBuf, status: git::FileStatus, window: &mut Window, cx: &mut Context<Self>) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let detail = if status == git::FileStatus::Added {
            "It's new since the last commit: it goes to the Trash."
        } else {
            "It goes back to how it was at the last commit."
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Discard the changes to {name}?"),
            Some(detail),
            &["Discard", "Cancel"],
            cx,
        );
        let root = self.tree.read(cx).root().to_path_buf();
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let reverting = path.clone();
            let result = cx.background_executor().spawn(async move { git::revert(&root, &reverting, status) }).await;
            this.update(cx, |this, cx| {
                match &result {
                    Ok(()) => this.show_notice(format!("Discarded the changes to {name}."), cx),
                    Err(error) => this.show_notice(format!("Couldn't discard them: {error}"), cx),
                }
                for tab in &this.tabs {
                    if result.is_ok() && tab.editor.read(cx).path().is_some_and(|p| same_file(p, &path)) {
                        tab.editor.update(cx, |editor, cx| {
                            editor.end_review(cx);
                            editor.revert_to_disk(cx);
                        });
                    }
                }
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
    }

    fn revert_all_changes(&mut self, _: &RevertAllChanges, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.git_status.len();
        if count == 0 {
            return;
        }
        let files = if count == 1 { "1 file".to_string() } else { format!("{count} files") };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Revert every change in {files}?"),
            Some("They go back to the last commit. New files go to the Trash."),
            &["Revert", "Cancel"],
            cx,
        );
        let changed = self.git_status.clone();
        let root = self.tree.read(cx).root().to_path_buf();
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let (reverted, failed) = cx
                .background_executor()
                .spawn(async move {
                    let (mut reverted, mut failed) = (Vec::new(), Vec::new());
                    for (path, status) in &changed {
                        match git::revert(&root, path, *status) {
                            Ok(()) => reverted.push(path.clone()),
                            Err(e) => failed.push(format!("{}: {e}", path.display())),
                        }
                    }
                    (reverted, failed)
                })
                .await;
            this.update(cx, |this, cx| {
                let notice = if failed.is_empty() { "Reverted every change.".to_string() } else { failed.join("; ") };
                this.show_notice(notice, cx);
                // Only the files that went back: edits not yet saved in any other stay.
                for tab in &this.tabs {
                    let was_reverted =
                        tab.editor.read(cx).path().is_some_and(|p| reverted.iter().any(|r| same_file(p, r)));
                    if was_reverted {
                        tab.editor.update(cx, |editor, cx| {
                            editor.end_review(cx);
                            editor.revert_to_disk(cx);
                        });
                    }
                }
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Points open tabs at their new location after a file or folder was renamed.
    /// Renames a file or folder, first letting language servers update the code that names
    /// it (`mod parser;`, imports). A server that's slow to answer doesn't hold it up long.
    fn rename_file(&mut self, from: PathBuf, to: PathBuf, cx: &mut Context<Self>) {
        const WAIT: Duration = Duration::from_secs(2);
        let request = self.lsp.read(cx).will_rename(&from, &to);
        // Markdown's links to it (and a moved Markdown file's own), which no server keeps.
        let root = self.tree.read(cx).root().to_path_buf();
        let (old, new) = (from.clone(), to.clone());
        // Open files as they are in their tabs, saved or not.
        let open: std::collections::HashMap<PathBuf, String> = self
            .tabs
            .iter()
            .filter_map(|t| {
                let editor = t.editor.read(cx);
                if !editor.is_markdown() {
                    return None;
                }
                Some((editor.path()?.to_path_buf(), editor.buffer.to_string()))
            })
            .collect();
        // Their versions now: one typed in while this waits isn't edited from the text read.
        let versions: std::collections::HashMap<PathBuf, u64> = self
            .tabs
            .iter()
            .filter_map(|t| {
                let editor = t.editor.read(cx);
                Some((editor.path()?.to_path_buf(), editor.buffer.version()))
            })
            .collect();
        let links = cx
            .background_executor()
            .spawn(async move { crate::markdown_links::edits_for_move(&root, &old, &new, &open) });
        cx.spawn(async move |this, cx| {
            let timeout = cx.background_executor().timer(WAIT);
            let mut edits = futures::select_biased! {
                edits = futures::FutureExt::fuse(request) => edits,
                _ = futures::FutureExt::fuse(timeout) => Vec::new(),
            };
            let links = links.await;
            this.update(cx, |this, cx| {
                let unchanged = |path: &Path| {
                    versions.get(path).is_none_or(|v| {
                        this.tabs
                            .iter()
                            .any(|t| t.editor.read(cx).path() == Some(path) && t.editor.read(cx).buffer.version() == *v)
                    })
                };
                edits.extend(links.map(|mut edit| {
                    if let Some(changes) = &mut edit.changes {
                        changes.retain(|uri, _| crate::lsp::path_for(uri).is_none_or(|p| unchanged(&p)));
                    }
                    edit
                }));
                // What every server asked for, as one: the files changed and those that couldn't be.
                let changed = (!edits.is_empty()).then(|| {
                    edits.into_iter().map(|edit| this.apply_edit_to_files(edit, cx)).fold(
                        (0, 0, Vec::new()),
                        |(places, files, mut failed), (p, f, mut x)| {
                            failed.append(&mut x);
                            (places + p, files + f, failed)
                        },
                    )
                });
                let result = this.tree.update(cx, |tree, cx| tree.finish_rename(&from, &to, cx));
                if let Err(error) = result {
                    return this.show_notice(error, cx);
                }
                this.lsp.read(cx).did_rename(&from, &to);
                match changed {
                    Some((_, _, failed)) if !failed.is_empty() => {
                        this.show_notice(format!("Renamed, but couldn't update {}", failed.join(", ")), cx)
                    }
                    Some((_, files, _)) if files > 0 => {
                        let which = if files == 1 { "1 file".to_string() } else { format!("{files} files") };
                        this.show_notice(format!("Renamed, and updated {which} that named it"), cx)
                    }
                    _ => {}
                }
            })
            .ok();
        })
        .detach();
    }

    /// Files moved outside Null (`mv`, `git mv`, another tool): an open file that's gone,
    /// and one with the same text that appeared with it, is the same file moved. Its tab
    /// follows, with the others from its folder when the folder moved.
    fn follow_moves(&mut self, changed: &[PathBuf], cx: &mut Context<Self>) {
        let open: Vec<PathBuf> =
            self.tabs.iter().filter_map(|t| t.editor.read(cx).path().map(Path::to_path_buf)).collect();
        let moves: Vec<(PathBuf, PathBuf)> = self
            .tabs
            .iter()
            .filter_map(|t| {
                let editor = t.editor.read(cx);
                let (true, Some(path), Some(print)) = (editor.missing, editor.path(), editor.on_disk) else {
                    return None;
                };
                moved_to(path, print, changed, &open)
            })
            .collect();
        for (from, to) in moves {
            // An earlier move (its folder's) may have taken this one along already.
            if self.tabs.iter().any(|t| t.editor.read(cx).path().is_some_and(|p| p.starts_with(&from))) {
                self.paths_renamed(&from, &to, cx);
            }
        }
        let active = self.active_editor().and_then(|e| e.read(cx).path().map(Path::to_path_buf));
        self.tree.update(cx, |tree, cx| tree.set_active(active, cx));
    }

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
        // Open on both sides: the copy on the side asked for (or being worked in).
        let wanted = side.unwrap_or_else(|| self.focused_side());
        let copies: Vec<usize> =
            (0..self.tabs.len()).filter(|&i| self.tabs[i].editor.read(cx).path() == Some(path.as_path())).collect();
        if let Some(&ix) = copies.iter().find(|&&i| self.tabs[i].side == wanted).or(copies.first()) {
            self.activate(ix, window, cx);
            return;
        }
        let lsp = self.lsp.clone();
        let editor = cx.new(|cx| Editor::open(path, Some(lsp), cx));
        self.add_tab_on(editor, side, window, cx);
    }

    /// A Markdown file's preview on the other side, kept up to date as you write in this one.
    fn open_preview_to_the_side(&mut self, _: &OpenPreviewToTheSide, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.active_editor().cloned() else { return };
        if !source.read(cx).is_markdown() {
            return self.show_notice("The preview is for Markdown files.".into(), cx);
        }
        self.open_on_other_side(window, cx);
        if let Some(twin) = self.twins_of(&source, cx).first() {
            twin.update(cx, |twin, cx| {
                twin.reading = true;
                cx.notify();
            });
        }
        // Writing goes on in the source.
        if let Some(at) = self.tabs.iter().position(|t| t.editor == source) {
            self.activate(at, window, cx);
        }
    }

    /// A Markdown preview beside its source follows it: the block at the source's top
    /// is at the preview's.
    fn sync_preview(&mut self, source: &Entity<Editor>, cx: &mut Context<Self>) {
        if source.read(cx).reading {
            return;
        }
        let line = source.read(cx).top_line();
        for twin in self.twins_of(source, cx) {
            if twin.read(cx).reading {
                twin.update(cx, |preview, cx| preview.follow_source_line(line, cx));
            }
        }
    }

    /// The other copies of a file open on both sides.
    fn twins_of(&self, editor: &Entity<Editor>, cx: &App) -> Vec<Entity<Editor>> {
        let Some(path) = editor.read(cx).path() else { return Vec::new() };
        self.tabs
            .iter()
            .filter(|t| &t.editor != editor && t.editor.read(cx).path() == Some(path))
            .map(|t| t.editor.clone())
            .collect()
    }

    /// ⌃⌥⌘→: the current file on the other side too, both copies kept the same as you type.
    fn open_on_other_side(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else { return };
        let source = self.tabs[ix].editor.clone();
        if source.read(cx).path().is_none() {
            return self.show_notice("Save the file first to open it on both sides".into(), cx);
        }
        let other = 1 - self.tabs[ix].side.min(1);
        if let Some(twin) = self.twins_of(&source, cx).first()
            && let Some(at) = self.tabs.iter().position(|t| &t.editor == twin)
        {
            return self.activate(at, window, cx);
        }
        self.add_twin(&source, other, window, cx);
    }

    fn add_twin(&mut self, source: &Entity<Editor>, side: usize, window: &mut Window, cx: &mut Context<Self>) {
        let lsp = self.lsp.clone();
        // An image has no text to share: the other side shows the file too.
        if source.read(cx).preview.is_some()
            && let Some(path) = source.read(cx).path().map(Path::to_path_buf)
        {
            let copy = cx.new(|cx| Editor::open(path, None, cx));
            return self.add_tab_on(copy, Some(side), window, cx);
        }
        let Some(from) = source.read(cx).twin_source() else { return };
        let twin = cx.new(|cx| Editor::twin(from, Some(lsp), cx));
        self.twin_seen.insert(source.entity_id(), source.read(cx).buffer.revision());
        self.twin_seen.insert(twin.entity_id(), twin.read(cx).buffer.revision());
        self.add_tab_on(twin, Some(side), window, cx);
    }

    /// Passes an edit in one copy of a file to its other copy.
    fn sync_twins(&mut self, source: &Entity<Editor>, cx: &mut Context<Self>) {
        let twins = self.twins_of(source, cx);
        if twins.is_empty() {
            return;
        }
        let (edits, text, revision, saved) = {
            let buffer = &source.read(cx).buffer;
            let since = self.twin_seen.get(&source.entity_id()).copied();
            let edits = since.and_then(|s| buffer.edits_since(s).map(|e| e.cloned().collect::<Vec<_>>()));
            (edits, buffer.rope().clone(), buffer.revision(), !buffer.is_dirty())
        };
        for twin in twins {
            twin.update(cx, |twin, cx| twin.apply_twin_edits(edits.clone(), &text, saved, cx));
            // What it just took from here isn't passed back.
            self.twin_seen.insert(twin.entity_id(), twin.read(cx).buffer.revision());
        }
        self.twin_seen.insert(source.entity_id(), revision);
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
            cx.observe(&editor, |this, editor, cx| {
                this.sync_preview(&editor, cx);
                cx.notify();
            }),
            cx.subscribe_in(&editor, window, |this, editor, event, window, cx| match event {
                EditorEvent::Edited => {
                    if let Some(path) = editor.read(cx).path() {
                        let place = Place { path: path.to_path_buf(), point: editor.read(cx).caret_point() };
                        this.last_edit = Some(place);
                    }
                    this.schedule_backup(cx);
                    if cx.global::<Settings>().auto_save == AutoSave::AfterPause {
                        this.save_after_pause(editor, cx);
                    }
                    this.sync_twins(editor, cx);
                    this.share_unsaved(editor, cx);
                    if cx.global::<Settings>().fade_bars_while_typing {
                        this.chrome.set(false, FADE_IN, FADE_OUT);
                    }
                    this.refresh_title(window, cx);
                    cx.notify();
                }
                // Hand edits to the settings file take effect when saved.
                EditorEvent::Saved => {
                    this.schedule_backup(cx);
                    this.refresh_git_status(cx);
                    // Saving (and tidying) one copy saves the other: same text, same file.
                    this.sync_twins(editor, cx);
                    for twin in this.twins_of(editor, cx) {
                        twin.update(cx, |twin, cx| {
                            twin.buffer.mark_saved();
                            cx.notify();
                        });
                    }
                    this.share_unsaved(editor, cx);
                    this.refresh_title(window, cx);
                    if editor.read(cx).path().is_some_and(|p| Some(p) == Settings::path().as_deref()) {
                        settings::reload(cx);
                    }
                    cx.notify();
                }
                EditorEvent::NeedsPath => this.ask_where_to_save(editor.clone(), window, cx),
                EditorEvent::Reviewed => this.file_reviewed(editor, cx),
                EditorEvent::FilesDropped { paths, at } => this.link_dropped(editor.clone(), paths, *at, cx),
                EditorEvent::SaveFailed(message) => this.show_notice(message.clone(), cx),
                EditorEvent::ChangedOnDisk => {
                    let name = editor.read(cx).file_name();
                    this.show_notice(format!("{name} changed on disk. Your unsaved edits were kept."), cx);
                }
                EditorEvent::SaveConflict => this.resolve_save_conflict(editor.clone(), window, cx),
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
                EditorEvent::CodeAction(fix) => {
                    let Some(path) = editor.read(cx).path().map(Path::to_path_buf) else { return };
                    this.run_fix(path, fix.clone(), cx);
                }
                EditorEvent::FindReferences { position, name } => {
                    let Some(path) = editor.read(cx).path().map(Path::to_path_buf) else { return };
                    let request = this.lsp.read(cx).references(&path, *position);
                    let name = name.clone();
                    cx.spawn_in(window, async move |this, cx| {
                        let found = request.await;
                        this.update_in(cx, |this, window, cx| {
                            let locations = this.location_rows(found, cx);
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
                EditorEvent::ShowLocations { title, locations } => {
                    let rows = this.location_rows(locations.clone(), cx);
                    this.open_locations(title.clone(), rows, window, cx);
                }
                EditorEvent::BreakpointsChanged => {
                    let editor = editor.read(cx);
                    if let Some(path) = editor.path().map(Path::to_path_buf) {
                        let lines = editor.breakpoint_list();
                        this.debugger.update(cx, |debugger, _| debugger.set_breakpoints(&path, &lines));
                    }
                    this.schedule_session_save(cx);
                }
                EditorEvent::GoTo { path, range } => {
                    let range = *range;
                    this.go_to(path.clone(), range, window, cx);
                }
                EditorEvent::Jumped { from } => {
                    if let Some(path) = editor.read(cx).path() {
                        let place = Place { path: path.to_path_buf(), point: *from };
                        this.remember_place(Some(place));
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

    /// The project's folder.
    pub fn root<'a>(&self, cx: &'a App) -> &'a Path {
        self.tree.read(cx).root()
    }

    fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // A folder another window has open: that window comes forward.
        if path.is_dir()
            && let Some(other) = crate::window_on(&path, Some(window.window_handle()), cx)
        {
            cx.defer(move |cx| {
                other.update(cx, |_, window, _| window.activate_window()).ok();
            });
            return;
        }
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
        // Leaving a file: Back comes back here, and it saves if files save when left.
        if let Some(leaving) = self.active_editor().filter(|e| **e != tab.editor).cloned() {
            let here = self.here(cx);
            self.remember_place(here);
            if cx.global::<Settings>().auto_save == AutoSave::WhenLeaving {
                Self::save_if_named(&leaving, cx);
            }
        }
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
        self.twin_seen.remove(&tab.editor.entity_id());
        // Its other copy, if any, talks to the language server now.
        if let Some(twin) = self.twins_of(&tab.editor, cx).first() {
            twin.update(cx, |twin, cx| twin.lead_lsp(cx));
        }
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
        // A copy of a file still open on the other side doesn't lose its changes by closing.
        let dirty: Vec<Entity<Editor>> = scope
            .iter()
            .filter(|e| e.read(cx).buffer.is_dirty())
            .filter(|e| !self.twins_of(e, cx).iter().any(|t| !scope.contains(t)))
            .cloned()
            .collect();
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
        // Every unsaved change was saved or let go: no backup to bring back.
        if !matches!(action, CloseAction::CloseTabs(_)) {
            self.backup_task = None;
            crate::session::save_backups(self.tree.read(cx).root(), &[]);
        }
        match action {
            // The other windows ask about theirs too, then Null quits.
            CloseAction::Quit => {
                self.quitting = true;
                cx.defer(crate::quit_next);
            }
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
                self.schedule_backup(cx);
            }
        }
    }

    // ---------- backups of unsaved work ----------

    /// Keeps the unsaved work in Null's own folder a moment after typing stops, so a
    /// crash or a forced quit doesn't lose it.
    fn schedule_backup(&mut self, cx: &mut Context<Self>) {
        const PAUSE: Duration = Duration::from_secs(2);
        self.backup_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE).await;
            this.update(cx, |this, cx| this.write_backups(cx)).ok();
        }));
    }

    /// Whether any open file has changes not saved.
    pub fn has_unsaved(&self, cx: &App) -> bool {
        self.tabs.iter().any(|t| t.editor.read(cx).buffer.is_dirty())
    }

    /// The unsaved files: path (None for a new one) and text, a file open twice once.
    fn unsaved_files(&self, cx: &App) -> Vec<(Option<PathBuf>, String)> {
        let mut unsaved: Vec<(Option<PathBuf>, String)> = Vec::new();
        for tab in &self.tabs {
            let editor = tab.editor.read(cx);
            let path = editor.path().map(Path::to_path_buf);
            if editor.buffer.is_dirty() && (path.is_none() || !unsaved.iter().any(|(p, _)| *p == path)) {
                unsaved.push((path, editor.buffer.to_string()));
            }
        }
        unsaved
    }

    fn backups_of(unsaved: Vec<(Option<PathBuf>, String)>) -> Vec<crate::session::Backup> {
        unsaved
            .into_iter()
            .map(|(path, text)| {
                let disk = path.as_deref().and_then(crate::session::disk_fingerprint);
                crate::session::Backup { path, text, disk }
            })
            .collect()
    }

    fn write_backups(&mut self, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        let unsaved = self.unsaved_files(cx);
        cx.background_executor()
            .spawn(async move { crate::session::save_backups(&root, &Self::backups_of(unsaved)) })
            .detach();
    }

    /// Null quitting without asking (logging out, the Mac restarting): unsaved work is
    /// kept now, not a moment later, so it comes back next time.
    fn write_backups_now(&mut self, cx: &mut Context<Self>) {
        if self.quitting {
            return;
        }
        let root = self.tree.read(cx).root().to_path_buf();
        crate::session::save_backups(&root, &Self::backups_of(self.unsaved_files(cx)));
    }

    /// ⌘⇧N: another window, on a folder chosen for it.
    fn new_window(&mut self, _: &NewWindow, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open in New Window".into()),
        });
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            if let Some(folder) = paths.into_iter().next() {
                cx.update(|_, cx| crate::open_project_window(folder, None, cx)).ok();
            }
        })
        .detach();
    }

    /// After opening a project: unsaved work left from last time comes back, unsaved.
    fn restore_backups(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        let backups = crate::session::load_backups(&root);
        if backups.is_empty() {
            return;
        }
        let mut changed_on_disk = 0;
        for backup in &backups {
            let editor = match &backup.path {
                Some(path) if path.is_file() => {
                    if backup.disk != crate::session::disk_fingerprint(path) {
                        changed_on_disk += 1;
                    }
                    self.open_file(path.clone(), window, cx);
                    self.tabs
                        .iter()
                        .find(|t| t.editor.read(cx).path() == Some(path.as_path()))
                        .map(|t| t.editor.clone())
                }
                // No file (a new one, or since deleted): a new file holding the text.
                _ => {
                    let editor = cx.new(|cx| Editor::new(Default::default(), None, cx));
                    self.add_tab(editor.clone(), window, cx);
                    Some(editor)
                }
            };
            let disk_changed = backup.path.as_ref().is_some_and(|p| backup.disk != crate::session::disk_fingerprint(p));
            if let Some(editor) = editor {
                editor.update(cx, |editor, cx| {
                    editor.restore_unsaved(&backup.text, cx);
                    // Written over a file that changed since: saving asks first.
                    editor.disk_changed |= disk_changed && editor.buffer.is_dirty();
                });
            }
        }
        let files = if backups.len() == 1 { "1 file".to_string() } else { format!("{} files", backups.len()) };
        let mut notice = format!("Brought back unsaved changes to {files} from last time");
        if changed_on_disk > 0 {
            notice.push_str(", though a file changed on disk since: check before saving");
        }
        self.show_notice(notice, cx);
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
        if self.active_editor().is_some_and(|e| e.read(cx).preview.is_some()) {
            return self.show_notice("Only text files are saved from Null.".into(), cx);
        }
        if let Some(editor) = self.active_editor().cloned() {
            self.ask_where_to_save(editor, window, cx);
        }
    }

    /// Asks for a location, then saves the editor there.
    /// Saving a file that changed on disk under unsaved edits: which version stays.
    fn resolve_save_conflict(&mut self, editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        let name = editor.read(cx).file_name();
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("{name} changed on disk since you started editing it."),
            Some("Saving writes your version over it. Reloading takes the one on disk, losing your edits."),
            &["Save Mine", "Reload From Disk", "Cancel"],
            cx,
        );
        cx.spawn(async move |_, cx| {
            let answer = answer.await;
            editor
                .update(cx, |editor, cx| match answer {
                    Ok(0) => {
                        editor.overwrite_disk(cx);
                    }
                    Ok(1) => editor.revert_to_disk(cx),
                    _ => {}
                })
                .ok();
        })
        .detach();
    }

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
            (File, "Open Recent…".into(), Box::new(OpenRecent)),
            (File, "New Window…".into(), Box::new(NewWindow)),
            (File, "Reopen Closed Tab".into(), Box::new(ReopenClosedTab)),
            (Go, "Go to File…".into(), Box::new(TogglePalette)),
            (Go, "Go to Symbol in Project…".into(), Box::new(GoToSymbolInProject)),
            (Go, "Search in Project…".into(), Box::new(SearchProject)),
            (Go, "Back".into(), Box::new(GoBack)),
            (Go, "Go to Last Edit".into(), Box::new(GoToLastEdit)),
            (Go, "Forward".into(), Box::new(GoForward)),
            (File, "Review Changes…".into(), Box::new(ReviewChanges)),
            (
                View,
                toggle(settings.line_blame, "Hide Who Changed the Line", "Show Who Changed the Line"),
                Box::new(ToggleLineBlame),
            ),
            (View, toggle(settings.inlay_hints, "Hide Type Hints", "Show Type Hints"), Box::new(ToggleInlayHints)),
            (
                View,
                toggle(settings.spell_check, "Hide Spelling Mistakes", "Show Spelling Mistakes"),
                Box::new(ToggleSpellCheck),
            ),
            (File, "Commit…".into(), Box::new(CommitAll)),
            (File, "Undo Last Commit".into(), Box::new(UndoLastCommit)),
            (File, "Set Changes Aside".into(), Box::new(SetChangesAside)),
            (File, "Bring Back Changes".into(), Box::new(BringBackChanges)),
            (File, "Push".into(), Box::new(PushBranch)),
            (File, "Pull".into(), Box::new(PullBranch)),
            (File, "Show File History".into(), Box::new(FileHistory)),
            (File, "Compare with Saved".into(), Box::new(CompareWithSaved)),
            (File, "Copy Link to Line".into(), Box::new(CopyLineLink)),
            (File, "Open Line on the Web".into(), Box::new(OpenLineOnWeb)),
            (File, "Fetch".into(), Box::new(FetchBranch)),
            (Go, "Find TODOs".into(), Box::new(FindTodos)),
            (Edit, "Copy as Code Block".into(), Box::new(CopyAsCodeBlock)),
            (File, "Rename File…".into(), Box::new(RenameFile)),
            (File, "Move File to Trash…".into(), Box::new(TrashFile)),
            (File, "Copy Path".into(), Box::new(CopyFilePath)),
            (File, "Copy Relative Path".into(), Box::new(CopyRelativeFilePath)),
            (File, crate::file_tree::REVEAL_LABEL.into(), Box::new(RevealFile)),
            (File, "Compare with Clipboard".into(), Box::new(CompareWithClipboard)),
            (File, "Compare with File…".into(), Box::new(CompareWithFile)),
            (File, "Switch Branch…".into(), Box::new(SwitchBranch)),
            (Edit, "Replace in Project…".into(), Box::new(ReplaceInProject)),
            (View, "Show Files".into(), Box::new(ShowFiles)),
            (View, toggle(settings.word_wrap, "Stop Wrapping Lines", "Wrap Lines"), Box::new(ToggleWordWrap)),
            (
                View,
                toggle(settings.fade_bars_while_typing, "Stop Fading Bars While Typing", "Fade Bars While Typing"),
                Box::new(ToggleFadeWhileTyping),
            ),
            (Appearance, theme_label(ThemeName::Null), theme_action(ThemeName::Null)),
            (Appearance, theme_label(ThemeName::Ash), theme_action(ThemeName::Ash)),
            (Appearance, theme_label(ThemeName::Midnight), theme_action(ThemeName::Midnight)),
            (Appearance, theme_label(ThemeName::Moss), theme_action(ThemeName::Moss)),
            (Appearance, theme_label(ThemeName::Paper), theme_action(ThemeName::Paper)),
            (Appearance, theme_label(ThemeName::Dune), theme_action(ThemeName::Dune)),
            (Appearance, "Bigger Text".into(), Box::new(IncreaseFontSize)),
            (Appearance, "Compact Line Spacing".into(), Box::new(CompactLineSpacing)),
            (Appearance, "Normal Line Spacing".into(), Box::new(NormalLineSpacing)),
            (Appearance, "Relaxed Line Spacing".into(), Box::new(RelaxedLineSpacing)),
            (Appearance, "Smaller Text".into(), Box::new(DecreaseFontSize)),
            (Appearance, "Actual Size".into(), Box::new(ResetFontSize)),
            (
                Edit,
                toggle(settings.autocomplete, "Turn Off Autocomplete", "Turn On Autocomplete"),
                Box::new(ToggleAutocomplete),
            ),
            (App, "Settings…".into(), Box::new(OpenSettings)),
            (View, toggle(self.focus_mode, "Leave Focus Mode", "Focus Mode"), Box::new(ToggleFocusMode)),
            (View, "Run Task…".into(), Box::new(RunTask)),
            (View, "Run Test at Cursor".into(), Box::new(RunTestAtCursor)),
            (View, "Toggle Markdown Preview".into(), Box::new(crate::editor::ToggleMarkdownPreview)),
            (View, "Open Markdown Preview to the Side".into(), Box::new(OpenPreviewToTheSide)),
            (View, "Run Tests in File".into(), Box::new(RunTestsInFile)),
            (Go, "Start Debugging".into(), Box::new(StartDebugging)),
            (Go, "Stop Debugging".into(), Box::new(StopDebugging)),
            (Edit, "Toggle Breakpoint".into(), Box::new(crate::editor::ToggleBreakpoint)),
            (
                View,
                toggle(settings.indent_guides, "Hide Indent Guides", "Show Indent Guides"),
                Box::new(ToggleIndentGuides),
            ),
            (
                View,
                toggle(settings.line_guide, "Hide Line-Length Guide", "Show Line-Length Guide"),
                Box::new(ToggleLineGuide),
            ),
            (
                View,
                toggle(settings.sticky_scroll, "Turn Off Sticky Scroll", "Turn On Sticky Scroll"),
                Box::new(ToggleStickyScroll),
            ),
            (
                View,
                toggle(settings.symbol_marks, "Stop Marking Other Uses of a Name", "Mark Other Uses of a Name"),
                Box::new(ToggleSymbolMarks),
            ),
            (Go, "Next Problem".into(), Box::new(NextProblem)),
            (Go, "Previous Problem".into(), Box::new(PreviousProblem)),
            (Go, "Next Change".into(), Box::new(crate::editor::NextChange)),
            (Go, "Previous Change".into(), Box::new(crate::editor::PreviousChange)),
            (Go, "Next Conflict".into(), Box::new(crate::editor::NextConflict)),
            (Go, "Previous Conflict".into(), Box::new(crate::editor::PreviousConflict)),
            (View, "Toggle Sidebar".into(), Box::new(ToggleSidebar)),
            (View, "Toggle Terminal".into(), Box::new(ToggleTerminal)),
            (View, "New Terminal".into(), Box::new(NewTerminal)),
            (View, "Next Terminal".into(), Box::new(NextTerminal)),
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
                (Lines, "Toggle Block Comment".into(), Box::new(crate::editor::ToggleBlockComment)),
                (Lines, "Indent (Tab on a selection)".into(), Box::new(crate::editor::Indent)),
                (Lines, "Outdent (Shift+Tab)".into(), Box::new(crate::editor::Outdent)),
                (Lines, "Move Line Up".into(), Box::new(crate::editor::MoveLineUp)),
                (Lines, "Move Line Down".into(), Box::new(crate::editor::MoveLineDown)),
                (Lines, "Duplicate Line".into(), Box::new(crate::editor::DuplicateLineDown)),
                (Lines, "Delete Line".into(), Box::new(crate::editor::DeleteLine)),
                (Lines, "Select Line".into(), Box::new(crate::editor::SelectLine)),
                (Lines, "Insert Line Below".into(), Box::new(crate::editor::NewlineBelow)),
                (Lines, "Insert Line Above".into(), Box::new(crate::editor::NewlineAbove)),
                (Lines, "Join Lines".into(), Box::new(crate::editor::JoinLines)),
                (Lines, "Sort Lines".into(), Box::new(crate::editor::SortLines)),
                (Lines, "Reverse Lines".into(), Box::new(crate::editor::ReverseLines)),
                (Lines, "Remove Duplicate Lines".into(), Box::new(crate::editor::RemoveDuplicateLines)),
                (Lines, "Rewrap Comment or Paragraph".into(), Box::new(crate::editor::Rewrap)),
                (Edit, "Upper Case".into(), Box::new(crate::editor::UpperCase)),
                (Edit, "To snake_case".into(), Box::new(crate::editor::SnakeCase)),
                (Edit, "To camelCase".into(), Box::new(crate::editor::CamelCase)),
                (Edit, "To PascalCase".into(), Box::new(crate::editor::PascalCase)),
                (Edit, "To kebab-case".into(), Box::new(crate::editor::KebabCase)),
                (Edit, "To Title Case".into(), Box::new(crate::editor::TitleCase)),
                (Edit, "Lower Case".into(), Box::new(crate::editor::LowerCase)),
                (Edit, "Expand Selection".into(), Box::new(crate::editor::ExpandSelection)),
                (Edit, "Shrink Selection".into(), Box::new(crate::editor::ShrinkSelection)),
                (Go, "Go to Matching Bracket".into(), Box::new(crate::editor::GoToMatchingBracket)),
                (Lines, "Indent with Tabs".into(), Box::new(crate::editor::IndentWithTabs)),
                (Lines, "Indent with 2 Spaces".into(), Box::new(crate::editor::IndentWith2Spaces)),
                (Lines, "Indent with 4 Spaces".into(), Box::new(crate::editor::IndentWith4Spaces)),
                (Lines, "Use LF Line Endings (macOS, Linux)".into(), Box::new(crate::editor::UseLfLineEndings)),
                (Lines, "Use CRLF Line Endings (Windows)".into(), Box::new(crate::editor::UseCrlfLineEndings)),
                (Lines, "Use UTF-8 Encoding".into(), Box::new(crate::editor::UseUtf8Encoding)),
                (Lines, "Fold".into(), Box::new(crate::editor::Fold)),
                (Lines, "Unfold".into(), Box::new(crate::editor::Unfold)),
                (Lines, "Fold All".into(), Box::new(crate::editor::FoldAll)),
                (Lines, "Unfold All".into(), Box::new(crate::editor::UnfoldAll)),
                (Cursors, "Add Next Occurrence".into(), Box::new(crate::editor::AddNextOccurrence)),
                (Cursors, "Select All Occurrences".into(), Box::new(crate::editor::SelectAllOccurrences)),
                (Cursors, "Add Cursor Above".into(), Box::new(crate::editor::AddCursorAbove)),
                (Cursors, "Add Cursor Below".into(), Box::new(crate::editor::AddCursorBelow)),
                (Cursors, "Add Cursors to Line Ends".into(), Box::new(crate::editor::AddCursorsToLineEnds)),
                (Cursors, "Undo Last Cursor".into(), Box::new(crate::editor::UndoCursor)),
                (Go, "Go to Line…".into(), Box::new(GoToLine)),
                (Go, "Go to Symbol…".into(), Box::new(GoToSymbol)),
                (Go, "Go to Definition".into(), Box::new(GoToDefinition)),
                (Go, "Peek Definition".into(), Box::new(crate::editor::PeekDefinition)),
                (Go, "Find References".into(), Box::new(crate::editor::FindReferences)),
                (Go, "Show Callers".into(), Box::new(crate::editor::ShowCallers)),
                (Go, "Go to Implementation".into(), Box::new(crate::editor::GoToImplementation)),
                (Go, "Go to Type Definition".into(), Box::new(crate::editor::GoToTypeDefinition)),
                (Go, "Show Problems".into(), Box::new(ShowProblems)),
                (Edit, "Rename Symbol".into(), Box::new(crate::editor::RenameSymbol)),
                (Edit, "Quick Fix…".into(), Box::new(crate::editor::QuickFix)),
                (Edit, "Format Document".into(), Box::new(crate::editor::FormatDocument)),
                (Edit, "Format Selection".into(), Box::new(crate::editor::FormatSelection)),
                (
                    Edit,
                    toggle(settings.format_on_save, "Stop Formatting on Save", "Format on Save"),
                    Box::new(ToggleFormatOnSave),
                ),
                (
                    File,
                    format!("Save Automatically After a Pause{}", current(settings.auto_save == AutoSave::AfterPause)),
                    Box::new(AutoSaveAfterPause),
                ),
                (
                    File,
                    format!(
                        "Save Automatically When Leaving a File{}",
                        current(settings.auto_save == AutoSave::WhenLeaving)
                    ),
                    Box::new(AutoSaveWhenLeaving),
                ),
                (
                    File,
                    format!("Don't Save Automatically{}", current(settings.auto_save == AutoSave::Off)),
                    Box::new(AutoSaveOff),
                ),
                (Go, "Show Info at Cursor".into(), Box::new(ShowInfo)),
                (Go, "Next Tab".into(), Box::new(NextTab)),
                (View, "Move Tab to the Right Side".into(), Box::new(MoveTabRight)),
                (View, "Move Tab to the Left Side".into(), Box::new(MoveTabLeft)),
                (View, "Open on the Other Side Too".into(), Box::new(OpenOnOtherSide)),
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
        // Markdown: a table of contents of its headings.
        if self.active_editor().is_some_and(|e| e.read(cx).is_markdown()) {
            commands.push((Edit, "Insert Table of Contents".into(), Box::new(crate::editor::InsertTableOfContents)));
        }
        // A page or a picture: open it in the browser.
        if self.active_editor().and_then(|e| e.read(cx).path()).is_some_and(crate::file_tree::opens_in_browser) {
            commands.push((File, "Open in Browser".into(), Box::new(OpenInBrowser)));
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
            prose_here: self.active_editor().is_some_and(|e| e.read(cx).is_prose()),
            title,
            locations,
            branches: std::mem::take(&mut self.pending_branches),
            tasks: std::mem::take(&mut self.pending_tasks),
            projects: std::mem::take(&mut self.pending_projects),
        };
        let palette = cx.new(|cx| Palette::new(options, cx));
        let subscription = cx.subscribe_in(&palette, window, |this, palette, event, window, cx| match event {
            PaletteEvent::Dismissed => this.close_palette(window, cx),
            PaletteEvent::SwitchBranch(branch) => {
                let branch = branch.clone();
                this.close_palette(window, cx);
                this.change_branch(Ok(branch), window, cx);
            }
            PaletteEvent::OpenProjectInNewWindow(path) => {
                let path = path.clone();
                this.close_palette(window, cx);
                crate::open_project_window(path, None, cx);
            }
            PaletteEvent::OpenProject(path) => {
                let path = path.clone();
                this.close_palette(window, cx);
                this.open_paths(vec![path], window, cx);
            }
            PaletteEvent::RunCommand(command) => {
                let command = command.clone();
                this.close_palette(window, cx);
                this.run_in_terminal(command, window, cx);
            }
            PaletteEvent::CreateBranch(name) => {
                let name = name.clone();
                this.close_palette(window, cx);
                this.change_branch(Err(name), window, cx);
            }
            PaletteEvent::WriteCommitMessage(files) => this.write_commit_message(files.clone(), cx),
            PaletteEvent::Commit(message, left_out) => {
                let (message, left_out) = (message.clone(), left_out.clone());
                this.close_palette(window, cx);
                this.commit_with(message, left_out, cx);
            }
            PaletteEvent::StartTask(text) => {
                let text = text.clone();
                this.close_palette(window, cx);
                this.start_task(text, window, cx);
            }
            PaletteEvent::OpenFile(path) => {
                let path = path.clone();
                let compare_from = this.compare_from.take();
                this.close_palette(window, cx);
                match compare_from {
                    Some(editor) => this.compare_with_file(&editor, &path, cx),
                    None => this.open_file(path, window, cx),
                }
            }
            PaletteEvent::OpenLocation(path, position) => {
                let (path, position) = (path.clone(), *position);
                // Going there: no going back to the view before the preview.
                this.view_before_preview = None;
                // What the list was, before closing it forgets.
                let (git_listing, history) = (this.git_listing, this.history.take());
                this.close_palette(window, cx);
                if let Some((file, past)) = history
                    && let Some(version) = past.get(position.line as usize)
                {
                    return match version {
                        Past::Commit(commit) => this.compare_with(&file, commit.clone(), window, cx),
                        Past::Saved(saved) => this.compare_with_version(&file, saved, window, cx),
                    };
                }
                if this.review_file(&path, window, cx) {
                    return;
                }
                if git_listing && this.review_git_file(&path, window, cx) {
                    return;
                }
                this.go_to(path, lsp_types::Range { start: position, end: position }, window, cx);
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
                let here = this.here(cx);
                this.remember_place(here);
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
        self.git_listing = false;
        self.history = None;
        // Leaving a symbol list without choosing: the view goes back to where it was.
        if let Some((editor, (line, column, top))) = self.view_before_preview.take() {
            editor.update(cx, |editor, cx| editor.restore_view(line, column, top, cx));
        }
        self.palette = None;
        self.settings_panel = None;
        self.compare_from = None;
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
                editor.set_text_size(px(settings.font_size), settings.line_spacing.factor(), cx);
                // Wrapping on or off moves everything: keep the caret in view.
                editor.autoscroll = true;
                cx.notify();
            });
        }
        menus::set(cx);
        cx.notify();
    }

    /// What project search starts with: the selection, or with nothing selected and nothing
    /// typed yet, the latest search in any file.
    fn project_search_text(&self, cx: &App) -> Option<String> {
        let selected =
            self.active_editor().map(|e| e.read(cx).selected_text()).filter(|t| !t.is_empty() && !t.contains('\n'));
        selected.or_else(|| {
            (!self.project_search.read(cx).has_query(cx)).then(|| crate::find_bar::latest_search(cx)).flatten()
        })
    }

    /// Every TODO, FIXME, HACK and XXX in the project, in project search.
    fn find_todos(&mut self, _: &FindTodos, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_search.set(true, SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        settings::update(cx, |s| s.sidebar_visible = true);
        let query = crate::project_search::todo_query();
        self.project_search.update(cx, |search, cx| search.search_for(query, window, cx));
        cx.notify();
    }

    fn search_project(&mut self, _: &SearchProject, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.project_search_text(cx);
        self.sidebar_search.set(true, SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        settings::update(cx, |s| s.sidebar_visible = true);
        self.project_search.update(cx, |search, cx| search.focus(selected, window, cx));
        cx.notify();
    }

    /// ⌘⇧H: project search with its replace field open.
    fn replace_in_project(&mut self, _: &ReplaceInProject, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.project_search_text(cx);
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
                    let Ok((text, encoding)) = crate::encoding::read(path) else { continue };
                    let edits = crate::project_search::replacements(&text, query, replacement, *line);
                    if edits.is_empty() {
                        continue;
                    }
                    let mut new = text.clone();
                    for (range, with) in edits.iter().rev() {
                        new.replace_range(range.clone(), with);
                    }
                    // Written back in its own encoding; one that can't hold the new text fails.
                    let written = crate::encoding::encode(&new, encoding)
                        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))
                        .and_then(|bytes| crate::fs_ops::write_file(path, &bytes));
                    match written {
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
                    .child(tab("files", "Files", !searching).active(|s| s.opacity(0.7)).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.show_files(&ShowFiles, window, cx)),
                    ))
                    .child(tab("search", "Search", searching).active(|s| s.opacity(0.7)).on_click(
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

    /// ⌥Z: wrapping on or off for the kind of file at hand, prose or code.
    fn toggle_word_wrap(&mut self, _: &ToggleWordWrap, _: &mut Window, cx: &mut Context<Self>) {
        if self.active_editor().is_some_and(|e| e.read(cx).is_prose()) {
            return settings::update(cx, |s| s.wrap_prose = !s.wrap_prose);
        }
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

    /// F8: to the next error or warning, in this file then the next ones (round to the
    /// first), with its message shown.
    fn next_problem(&mut self, _: &NextProblem, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to_problem(true, window, cx);
    }

    fn previous_problem(&mut self, _: &PreviousProblem, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to_problem(false, window, cx);
    }

    fn go_to_problem(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        use lsp_types::DiagnosticSeverity as S;
        let mut places = self.problem_places(cx);
        places.retain(|(_, d)| matches!(d.severity.unwrap_or(S::ERROR), S::ERROR | S::WARNING));
        let key = |path: &Path, p: lsp_types::Position| (path.to_path_buf(), p.line, p.character);
        let mut places: Vec<_> = places.into_iter().map(|(path, d)| (key(&path, d.range.start), d.range)).collect();
        places.sort_by(|a, b| a.0.cmp(&b.0));
        places.dedup_by(|a, b| a.0 == b.0);
        if places.is_empty() {
            return self.show_notice("No problems found".into(), cx);
        }
        let here = self.active_editor().and_then(|e| {
            let e = e.read(cx);
            let (line, col) = e.caret_point();
            let character = e.buffer.column_to_utf16(line, col) as u32;
            Some((e.path()?.to_path_buf(), line as u32, character))
        });
        let next = match &here {
            Some(here) if forward => places.iter().find(|(k, _)| k > here).or(places.first()),
            Some(here) => places.iter().rev().find(|(k, _)| k < here).or(places.last()),
            None => places.first(),
        };
        let Some(((path, _, _), range)) = next.cloned() else { return };
        let start = lsp_types::Range { start: range.start, end: range.start };
        self.go_to(path, start, window, cx);
        if let Some(editor) = self.active_editor() {
            editor.update(cx, |editor, cx| editor.show_info_now(cx));
        }
    }

    /// Every problem the servers report, where it is now: open files say where their
    /// problems moved to while editing.
    fn problem_places(&self, cx: &App) -> Vec<(PathBuf, lsp_types::Diagnostic)> {
        let mut open: Vec<(PathBuf, Vec<lsp_types::Diagnostic>)> = Vec::new();
        for tab in &self.tabs {
            let editor = tab.editor.read(cx);
            // A file open on both sides counts once.
            if let Some(path) = editor.path().filter(|p| !open.iter().any(|(o, _)| o == p))
                && let Some(list) = editor.current_diagnostics(cx)
            {
                open.push((path.to_path_buf(), list));
            }
        }
        let mut found: Vec<(PathBuf, lsp_types::Diagnostic)> = self
            .lsp
            .read(cx)
            .all_diagnostics()
            .filter(|(p, _)| !open.iter().any(|(o, _)| o == *p))
            .map(|(p, d)| (p.clone(), d.clone()))
            .collect();
        for (path, list) in open {
            found.extend(list.into_iter().map(|d| (path.clone(), d)));
        }
        found
    }

    // ---------- debugging ----------

    /// F5: start debugging, or carry on from a stop.
    fn start_debugging(&mut self, _: &StartDebugging, window: &mut Window, cx: &mut Context<Self>) {
        use crate::debugger::DebugState;
        match self.debugger.read(cx).state {
            DebugState::Stopped(_) => return self.debugger.update(cx, |d, cx| d.resume("continue", cx)),
            DebugState::Idle => {}
            _ => return,
        }
        self.save_named_tabs(cx);
        self.debug_panel_open = true;
        let root = self.tree.read(cx).root().to_path_buf();
        if root.join("Cargo.toml").is_file() {
            return self.build_and_debug(root, cx);
        }
        match self.debug_program.clone().filter(|p| p.is_file()) {
            Some(program) => self.debug(program, cx),
            None => self.choose_program_to_debug(window, cx),
        }
    }

    /// Without a build Null knows: asks which program to run (once per project).
    fn choose_program_to_debug(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Debug".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(program) = paths.into_iter().next() else { return };
            this.update(cx, |this, cx| {
                this.debug_program = Some(program.clone());
                this.debug(program, cx);
            })
            .ok();
        })
        .detach();
    }

    /// A Rust project: `cargo build`, then its program under the debugger.
    fn build_and_debug(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let program = cargo_program(&root);
        self.debugger.update(cx, |d, cx| {
            d.set_building(program.clone(), cx);
            d.append_output("cargo build\n", cx);
        });
        cx.spawn(async move |this, cx| {
            let cwd = root.clone();
            let built = cx
                .background_executor()
                .spawn(async move {
                    let cargo = crate::tools::find("cargo").unwrap_or_else(|| PathBuf::from("cargo"));
                    let built = std::process::Command::new(cargo).arg("build").current_dir(&cwd).output();
                    // Rust's own formatters, so its strings and collections show their contents.
                    (built, crate::debugger::rust_formatters())
                })
                .await;
            let (built, formatters) = built;
            this.update(cx, |this, cx| {
                let message = match (&built, &program) {
                    (Ok(output), _) if !output.status.success() => {
                        Some(String::from_utf8_lossy(&output.stderr).into_owned() + "\nThe build failed.\n")
                    }
                    (Err(error), _) => Some(format!("Couldn't run cargo: {error}\n")),
                    (_, None) => Some("Couldn't tell which program this project builds.\n".into()),
                    (_, Some(p)) if !p.is_file() => Some(format!("{} wasn't built.\n", p.display())),
                    _ => None,
                };
                match message {
                    Some(message) => this.debugger.update(cx, |d, cx| {
                        d.append_output(&message, cx);
                        d.stop(cx);
                    }),
                    None => this.debug_with(program.expect("checked above"), formatters.into_iter().collect(), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Runs `program` under the debugger, with every breakpoint set in the open files.
    fn debug(&mut self, program: PathBuf, cx: &mut Context<Self>) {
        self.debug_with(program, Vec::new(), cx);
    }

    fn debug_with(&mut self, program: PathBuf, init_commands: Vec<String>, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        let mut breakpoints: Vec<(PathBuf, Vec<crate::editor::Breakpoint>)> = Vec::new();
        for tab in &self.tabs {
            let editor = tab.editor.read(cx);
            if let Some(path) = editor.path().filter(|_| !editor.breakpoints.is_empty())
                && !breakpoints.iter().any(|(p, _)| p == path)
            {
                breakpoints.push((path.to_path_buf(), editor.breakpoint_list()));
            }
        }
        self.debugger.update(cx, |d, cx| d.start(program, root, breakpoints, init_commands, cx));
    }

    fn debugger_event(&mut self, event: &crate::debugger::DebuggerEvent, window: &mut Window, cx: &mut Context<Self>) {
        use crate::debugger::DebuggerEvent;
        for tab in &self.tabs {
            tab.editor.update(cx, |e, cx| {
                let had_values = !e.inline_values.is_empty();
                e.inline_values.clear();
                e.debug_locals.clear();
                if e.execution_line.take().is_some() || had_values {
                    cx.notify();
                }
            });
        }
        // Unless the paused call's code is open, every variable with a value worth showing.
        self.debug_locals = match event {
            DebuggerEvent::Stopped(stop) => stop
                .locals
                .iter()
                .filter_map(|(name, value)| Some((name.clone(), crate::debugger::clean_value(value)?)))
                .collect(),
            _ => Vec::new(),
        };
        if let DebuggerEvent::Stopped(stop) = event
            && let Some((path, line)) = stop.place.clone()
        {
            let mut shown = None;
            // Shown, not gone to: stepping isn't somewhere for Back to return to each time.
            self.navigating = true;
            self.open_file(path.clone(), window, cx);
            self.navigating = false;
            for tab in &self.tabs {
                if tab.editor.read(cx).path() == Some(path.as_path()) {
                    tab.editor.update(cx, |e, cx| {
                        e.execution_line = Some(line);
                        let indent = e.buffer.line_text(line).chars().take_while(|c| c.is_whitespace()).count();
                        e.set_caret_point((line, indent), cx);
                        // The lines of this call so far (from its start, at most 40 back).
                        const BACK: usize = 40;
                        let start = e.blocks_around(line).last().map_or(0, |b| b.start).max(line.saturating_sub(BACK));
                        let lines: Vec<(usize, String)> = (start..=line).map(|l| (l, e.buffer.line_text(l))).collect();
                        e.inline_values = crate::debugger::inline_values(&lines, &stop.locals);
                        e.debug_locals = stop
                            .locals
                            .iter()
                            .filter_map(|(n, v)| Some((n.clone(), crate::debugger::clean_value(v)?)))
                            .collect();
                        shown = Some(crate::debugger::shown_locals(&lines, &stop.locals));
                    });
                }
            }
            if let Some(shown) = shown {
                self.debug_locals = shown;
            }
        }
        cx.notify();
    }

    /// The Debug panel: where things stand, the controls, and what the program printed.
    fn render_debug_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::debugger::DebugState;
        let debugger = self.debugger.read(cx);
        if !debugger.is_active() && !self.debug_panel_open {
            return None;
        }
        let theme = cx.global::<Theme>().clone();
        let root = self.tree.read(cx).root().to_path_buf();
        let status = match &debugger.state {
            DebugState::Idle => "Ended".to_string(),
            DebugState::Building => "Building…".to_string(),
            DebugState::Starting => "Starting…".to_string(),
            DebugState::Running => "Running".to_string(),
            DebugState::Stopped(stop) => {
                let place = stop.place.as_ref().map(|(p, l)| {
                    let file = p.strip_prefix(&root).unwrap_or(p).display().to_string();
                    format!(" at {file}:{}", l + 1)
                });
                let reason = match stop.reason.as_str() {
                    "breakpoint" => " (breakpoint)".to_string(),
                    "exception" => format!(" ({})", stop.description.clone().unwrap_or("exception".into())),
                    _ => String::new(),
                };
                format!("Paused{}{reason}", place.unwrap_or_default())
            }
        };
        let button = |id: &'static str, label: &'static str, tip: &'static str, action: Box<dyn Action>| {
            let run = action.boxed_clone();
            div()
                .id(id)
                .px(px(8.))
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(ui::R_KEY))
                .cursor_pointer()
                .text_color(theme.muted)
                .hover(|s| s.text_color(theme.foreground).bg(theme.hairline))
                .tooltip(ui::tip(tip, Some(action)))
                .child(label)
                .active(|s| s.opacity(0.7))
                .on_click(move |_, window, cx| window.dispatch_action(run.boxed_clone(), cx))
        };
        let controls: Vec<gpui::Stateful<gpui::Div>> = match &debugger.state {
            DebugState::Stopped(_) => vec![
                button("dbg-continue", "Continue", "Continue", Box::new(StartDebugging)),
                button("dbg-over", "Step Over", "Step over", Box::new(StepOver)),
                button("dbg-into", "Into", "Step into", Box::new(StepInto)),
                button("dbg-out", "Out", "Step out", Box::new(StepOut)),
                button("dbg-stop", "Stop", "Stop debugging", Box::new(StopDebugging)),
            ],
            DebugState::Running => vec![
                button("dbg-pause", "Pause", "Pause", Box::new(PauseDebugging)),
                button("dbg-stop", "Stop", "Stop debugging", Box::new(StopDebugging)),
            ],
            DebugState::Idle => vec![button("dbg-again", "Debug Again", "Start debugging", Box::new(StartDebugging))],
            _ => vec![button("dbg-stop", "Stop", "Stop debugging", Box::new(StopDebugging))],
        };
        let close = div()
            .id("close-debug")
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(ui::R_KEY))
            .cursor_pointer()
            .hover(|s| s.bg(theme.hairline))
            .child(svg().path("icons/x.svg").size(px(12.)).text_color(theme.muted))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.debugger.update(cx, |d, cx| d.stop(cx));
                this.debug_panel_open = false;
                cx.notify();
            }));
        let stopped = match &debugger.state {
            DebugState::Stopped(stop) => Some(stop.clone()),
            _ => None,
        };
        let showing_output = stopped.is_none() || self.debug_show_output;
        // While paused: variables and calls, or the output, a click apart.
        let view_switch = stopped.as_ref().map(|_| {
            div()
                .id("dbg-view")
                .px(px(8.))
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(ui::R_KEY))
                .cursor_pointer()
                .text_color(theme.faint)
                .hover(|s| s.text_color(theme.foreground))
                .child(if showing_output { "Variables" } else { "Output" })
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.debug_show_output = !this.debug_show_output;
                    cx.notify();
                }))
        });
        let header = div()
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(12.))
            .text_size(px(ui::T_SM))
            .child(div().text_color(theme.foreground).pr(px(6.)).child("Debug"))
            .child(div().flex_1().min_w_0().truncate().text_color(theme.muted).child(status))
            .children(view_switch)
            .children(controls)
            .child(close);
        let body = match stopped.filter(|_| !showing_output) {
            // Paused: the call's variables, and the calls that led to it (a click looks at one).
            Some(stop) => {
                let variables = self.debug_locals.iter().map(|(name, value)| {
                    div()
                        .flex()
                        .gap(px(10.))
                        .whitespace_nowrap()
                        .child(div().text_color(theme.foreground).child(name.clone()))
                        .child(div().min_w_0().truncate().text_color(theme.muted).child(value.clone()))
                });
                // The project's own calls; the libraries' ones (the runtime, std) are only counted.
                let in_project =
                    |f: &crate::debugger::Frame| f.place.as_ref().is_some_and(|(p, _)| p.starts_with(&root));
                let hidden = stop.frames.iter().filter(|f| !in_project(f)).count();
                let calls = stop.frames.iter().enumerate().filter(|(_, f)| in_project(f)).map(|(i, frame)| {
                    let place = frame.place.as_ref().map(|(p, l)| {
                        let file = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        format!("{file}:{}", l + 1)
                    });
                    let looked_at = i == stop.frame;
                    let has_source = frame.place.is_some();
                    div()
                        .id(("dbg-frame", i))
                        .flex()
                        .gap(px(10.))
                        .whitespace_nowrap()
                        .when(has_source, |d| d.cursor_pointer().hover(|s| s.text_color(theme.foreground)))
                        .text_color(if looked_at {
                            theme.caret
                        } else if has_source {
                            theme.muted
                        } else {
                            theme.faint
                        })
                        .child(div().min_w_0().truncate().child(frame.name.clone()))
                        .children(place.map(|p| div().flex_none().text_color(theme.faint).child(p)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if has_source {
                                this.debugger.update(cx, |d, cx| d.select_frame(i, cx));
                            }
                        }))
                });
                let column = |id: &'static str| {
                    div().id(id).flex_1().min_w_0().h_full().overflow_y_scroll().flex().flex_col().gap(px(3.))
                };
                div()
                    .id("debug-paused")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap(px(24.))
                    .px(px(14.))
                    .pb(px(8.))
                    .code_font(cx)
                    .text_size(px(12.5))
                    .child(column("dbg-variables").children(variables).when(self.debug_locals.is_empty(), |d| {
                        d.child(div().text_color(theme.faint).child("No local variables here"))
                    }))
                    .child(column("dbg-calls").children(calls).when(hidden > 0, |d| {
                        let more = if hidden == 1 {
                            "1 more in libraries".to_string()
                        } else {
                            format!("{hidden} more in libraries")
                        };
                        d.child(div().text_color(theme.faint).child(more))
                    }))
            }
            None => {
                let output = debugger.output.lines().map(|line| div().child(line.to_string()));
                div()
                    .id("debug-output")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.debug_output_scroll)
                    .px(px(14.))
                    .pb(px(8.))
                    .code_font(cx)
                    .text_size(px(12.5))
                    .text_color(theme.muted)
                    .children(output)
            }
        };
        Some(
            div()
                .flex_none()
                .h(px(TERMINAL_HEIGHT * 0.8))
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(theme.hairline)
                .bg(theme.background)
                .child(header)
                .child(body)
                .into_any_element(),
        )
    }

    /// ⌥⌘O: projects opened lately (not this one), to switch to.
    fn open_recent(&mut self, _: &OpenRecent, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        let projects: Vec<PathBuf> = crate::session::recent_projects().into_iter().filter(|p| *p != root).collect();
        if projects.is_empty() {
            return self.show_notice("No other projects opened lately".into(), cx);
        }
        self.pending_projects = projects;
        self.open_palette_with(PaletteKind::Projects, None, Vec::new(), window, cx);
    }

    /// ⌘⇧B: the project's tasks (the last run first), to run one in the terminal.
    fn run_task(&mut self, _: &RunTask, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        let mut tasks = crate::tasks::find(&root);
        // Run lately: first, whether the project lists them or they were typed.
        for command in self.recent_runs.iter().rev() {
            let task = match tasks.iter().position(|t| &t.command == command) {
                Some(i) => tasks.remove(i),
                None => {
                    crate::tasks::ProjectTask { label: command.clone(), command: command.clone(), source: "run lately" }
                }
            };
            tasks.insert(0, task);
        }
        self.pending_tasks = tasks;
        self.open_palette_with(PaletteKind::Run, None, Vec::new(), window, cx);
    }

    /// ⌥⌘T: the test the caret is in (or every test in the file) runs in the terminal,
    /// open files saved first so it runs what's on screen.
    fn run_test(&mut self, at_caret: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        let root = self.tree.read(cx).root().to_path_buf();
        let Some(run) = editor.update(cx, |e, _| e.test_run(&root, at_caret)) else {
            let message = if at_caret { "No test at the caret." } else { "No tests in this file." };
            return self.show_notice(message.into(), cx);
        };
        for tab in &self.tabs {
            if tab.editor.read(cx).buffer.is_dirty() && tab.editor.read(cx).path().is_some() {
                tab.editor.update(cx, |e, cx| e.save_to_disk(cx));
            }
        }
        self.show_notice(format!("Running {}", run.name), cx);
        self.run_in_terminal(run.command, window, cx);
    }

    /// Types `command` into the terminal (opening it first if needed) and runs it.
    fn run_in_terminal(&mut self, command: String, window: &mut Window, cx: &mut Context<Self>) {
        self.recent_runs.retain(|c| c != &command);
        self.recent_runs.insert(0, command.clone());
        self.recent_runs.truncate(10);
        if !self.terminal_open.on {
            self.toggle_terminal(&ToggleTerminal, window, cx);
        }
        if let Some(terminal) = self.terminal().cloned() {
            terminal.update(cx, |terminal, cx| terminal.run_command(&command, cx));
            window.focus(&terminal.focus_handle(cx));
        }
    }

    /// The terminal shown in the panel.
    fn terminal(&self) -> Option<&Entity<TerminalView>> {
        self.terminals.get(self.active_terminal).map(|(t, _)| t)
    }

    /// Starts a shell in the project folder, as the terminal shown.
    fn add_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let root = self.tree.read(cx).root().to_path_buf();
        self.add_terminal_in(root, window, cx)
    }

    /// A terminal in folder `dir`, opened and focused (the tree's Open in Terminal).
    fn open_terminal_in(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !self.add_terminal_in(dir, window, cx) {
            return;
        }
        self.terminal_open.set(true, TERMINAL_SLIDE, TERMINAL_SLIDE);
        if let Some(terminal) = self.terminal() {
            window.focus(&terminal.focus_handle(cx));
        }
        cx.notify();
    }

    fn add_terminal_in(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let shell = match Shell::start(dir) {
            Ok(shell) => shell,
            Err(err) => {
                self.show_notice(format!("Couldn't start a terminal: {err}"), cx);
                return false;
            }
        };
        let terminal = cx.new(|cx| TerminalView::new(shell, cx));
        let subscription = cx.subscribe_in(&terminal, window, |this, terminal, event, window, cx| match event {
            TerminalEvent::TitleChanged => cx.notify(),
            // The shell exited (e.g. `exit`): its tab goes; with none left, so does the panel.
            TerminalEvent::Exited => this.remove_terminal(terminal.entity_id(), window, cx),
            TerminalEvent::OpenFile(path, line, column) => {
                this.open_file(path.clone(), window, cx);
                if let Some(editor) = this.active_editor().cloned() {
                    let place =
                        (line.unwrap_or(1).saturating_sub(1) as usize, column.unwrap_or(1).saturating_sub(1) as usize);
                    editor.update(cx, |e, cx| e.set_caret_point(place, cx));
                    window.focus(&editor.focus_handle(cx));
                }
            }
        });
        self.terminals.push((terminal, subscription));
        self.active_terminal = self.terminals.len() - 1;
        true
    }

    fn remove_terminal(&mut self, id: gpui::EntityId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.terminals.iter().position(|(t, _)| t.entity_id() == id) else { return };
        // Its subscription goes with it.
        drop(self.terminals.remove(ix));
        if self.active_terminal > ix || self.active_terminal >= self.terminals.len() {
            self.active_terminal = self.active_terminal.saturating_sub(1);
        }
        match self.terminal().cloned() {
            Some(next) if self.terminal_open.on => window.focus(&next.focus_handle(cx)),
            Some(_) => {}
            None => {
                self.terminal_open.set(false, TERMINAL_SLIDE, TERMINAL_SLIDE);
                self.focus_main(window, cx);
            }
        }
        cx.notify();
    }

    /// ⌃⇧`: another terminal, beside the ones open.
    fn new_terminal(&mut self, _: &NewTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if !self.add_terminal(window, cx) {
            return;
        }
        self.terminal_open.set(true, TERMINAL_SLIDE, TERMINAL_SLIDE);
        if let Some(terminal) = self.terminal() {
            window.focus(&terminal.focus_handle(cx));
        }
        cx.notify();
    }

    fn show_terminal(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.terminals.len() {
            self.active_terminal = ix;
            if let Some(terminal) = self.terminal() {
                window.focus(&terminal.focus_handle(cx));
            }
            cx.notify();
        }
    }

    fn next_terminal(&mut self, _: &NextTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if !self.terminals.is_empty() {
            self.show_terminal((self.active_terminal + 1) % self.terminals.len(), window, cx);
        }
    }

    fn toggle_terminal(&mut self, _: &ToggleTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_open.on {
            let had_focus = self.terminal().is_some_and(|t| t.focus_handle(cx).is_focused(window));
            self.terminal_open.set(false, TERMINAL_SLIDE, TERMINAL_SLIDE);
            if had_focus {
                self.focus_main(window, cx);
            }
            return cx.notify();
        }
        if self.terminals.is_empty() && !self.add_terminal(window, cx) {
            return;
        }
        self.terminal_open.set(true, TERMINAL_SLIDE, TERMINAL_SLIDE);
        if let Some(terminal) = self.terminal() {
            window.focus(&terminal.focus_handle(cx));
        }
        cx.notify();
    }

    /// A terminal's tab: its shell's folder, from the title it sets ("user@host:~/a/b" → "b").
    fn terminal_label(title: &str, ix: usize) -> String {
        let name = Self::terminal_name(title);
        if name.is_empty() { format!("Terminal {}", ix + 1) } else { name.to_string() }
    }

    /// What a shell's title comes down to: the folder it's in ("IDE" from
    /// "me@mac:~/code/IDE"), or the command it runs.
    fn terminal_name(title: &str) -> &str {
        let path = title.rsplit(':').next().unwrap_or(title).trim();
        path.trim_end_matches('/').rsplit('/').next().unwrap_or("").trim()
    }

    fn render_terminal_panel(&self, height: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let terminal = self.terminal()?;
        if height < 0.5 {
            return None;
        }
        let theme = cx.global::<Theme>();
        let title = Self::terminal_name(&terminal.read(cx).title).to_string();
        let icon_button = |id: &'static str, icon: &'static str| {
            div()
                .id(id)
                .size(px(20.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(ui::R_KEY))
                .cursor_pointer()
                .group(id)
                .hover(|s| s.bg(theme.hairline))
                .child(
                    svg()
                        .path(icon)
                        .size(px(12.))
                        .text_color(theme.muted)
                        .group_hover(id, |s| s.text_color(theme.foreground)),
                )
                .active(|s| s.opacity(0.7))
        };
        // One terminal: its name and title. More: a tab each.
        let several = self.terminals.len() > 1;
        let tabs = self.terminals.iter().enumerate().map(|(ix, (t, _))| {
            let active = ix == self.active_terminal;
            let label = Self::terminal_label(&t.read(cx).title, ix);
            div()
                .id(("terminal-tab", ix))
                .h(px(22.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(ui::R_KEY))
                .cursor_pointer()
                .whitespace_nowrap()
                .text_color(if active { theme.foreground } else { theme.muted })
                .when(active, |d| d.bg(theme.hairline))
                .when(!active, |d| d.hover(|s| s.text_color(theme.foreground)))
                .child(label)
                .active(|s| s.opacity(0.7))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.show_terminal(ix, window, cx)))
        });
        let header = div()
            .h(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(12.))
            .text_size(px(ui::T_SM))
            .text_color(theme.muted)
            .map(|bar| {
                if several {
                    bar.children(tabs).child(div().flex_1())
                } else {
                    bar.child(div().text_color(theme.foreground).child("Terminal"))
                        .child(div().flex_1().min_w_0().truncate().text_color(theme.muted).child(title))
                }
            })
            .child(
                icon_button("new-terminal", "icons/plus.svg")
                    .tooltip(ui::tip("New terminal", Some(Box::new(NewTerminal))))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.new_terminal(&NewTerminal, window, cx)),
                    ),
            )
            .child(
                icon_button("close-terminal", "icons/x.svg")
                    .tooltip(ui::tip("Hide the terminal", Some(Box::new(ToggleTerminal))))
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
        crate::encoding::read(path).ok().and_then(|(t, _)| t.lines().nth(line).map(str::to_string)).unwrap_or_default()
    }

    /// Applies a rename (or any multi-file change) from a language server: open files as
    /// one undo step each, others written on disk.
    /// A quick fix: its edits (worked out now if the server left them for later), then
    /// its command, which may send more edits.
    fn run_fix(&mut self, path: PathBuf, fix: lsp_types::CodeActionOrCommand, cx: &mut Context<Self>) {
        let lsp = self.lsp.clone();
        cx.spawn(async move |this, cx| {
            let (edit, command) = match fix {
                lsp_types::CodeActionOrCommand::Command(command) => (None, Some(command)),
                lsp_types::CodeActionOrCommand::CodeAction(action) => {
                    let action = if action.edit.is_none() && action.data.is_some() {
                        let Ok(request) = this.update(cx, |_, cx| lsp.read(cx).resolve_code_action(&path, action))
                        else {
                            return;
                        };
                        match request.await {
                            Ok(action) => action,
                            Err(error) => {
                                this.update(cx, |this, cx| this.show_notice(format!("Couldn't fix it: {error}"), cx))
                                    .ok();
                                return;
                            }
                        }
                    } else {
                        action
                    };
                    (action.edit, action.command)
                }
            };
            let Ok(request) = this.update(cx, |this, cx| {
                if let Some(edit) = edit {
                    this.apply_fix_edit(edit, cx);
                }
                command.map(|command| lsp.read(cx).execute_command(&path, command))
            }) else {
                return;
            };
            if let Some(request) = request {
                // Commands only some editors have (opening a view, say) just fail: say nothing.
                request.await.ok();
            }
        })
        .detach();
    }

    /// A fix's edits: quiet when they stay in the open file.
    fn apply_fix_edit(&mut self, edit: lsp_types::WorkspaceEdit, cx: &mut Context<Self>) {
        let (_, files, failed) = self.apply_edit_to_files(edit, cx);
        if !failed.is_empty() {
            self.show_notice(format!("Couldn't write {}", failed.join(", ")), cx);
        } else if files > 1 {
            self.show_notice(format!("Changed {files} files"), cx);
        }
    }

    fn apply_workspace_edit(&mut self, edit: lsp_types::WorkspaceEdit, cx: &mut Context<Self>) {
        let (places, files, failed) = self.apply_edit_to_files(edit, cx);
        let message = match (places, failed.is_empty()) {
            (0, _) => "Nothing to rename".to_string(),
            (_, true) if files == 1 => format!("Renamed in {places} places"),
            (_, true) => format!("Renamed in {places} places across {files} files"),
            (_, false) => format!("Renamed, but couldn't write {}", failed.join(", ")),
        };
        self.show_notice(message, cx);
    }

    /// Applies edits to every file they touch: open ones in their tab (to undo), others
    /// on disk. Returns how many places and files changed, and the files it couldn't write.
    fn apply_edit_to_files(
        &mut self,
        edit: lsp_types::WorkspaceEdit,
        cx: &mut Context<Self>,
    ) -> (usize, usize, Vec<String>) {
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
                let written = crate::encoding::read(&path).and_then(|(text, encoding)| {
                    let mut buffer = crate::buffer::Buffer::from_text(&text);
                    crate::editor::apply_edits(&mut buffer, &edits);
                    let bytes = crate::encoding::encode(&buffer.to_string(), encoding)
                        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
                    crate::fs_ops::write_file(&path, &bytes)
                });
                if written.is_err() {
                    failed.push(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                }
            }
        }
        (places, files, failed)
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
        let found = self.problem_places(cx);
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

    /// ⌥⌘↵: only the code, or everything back.
    fn toggle_focus_mode(&mut self, _: &ToggleFocusMode, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_mode = !self.focus_mode;
        if self.focus_mode {
            let keys = crate::palette::shortcut(&ToggleFocusMode, cx).unwrap_or_default();
            self.show_notice(format!("Focus mode · {keys} to come back"), cx);
        }
        self.focus_main(window, cx);
        cx.notify();
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
                        .active(|s| s.opacity(0.7))
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
                // On a problem: say how to fix it.
                if self.active_editor().is_some_and(|e| e.read(cx).caret_on_problem(cx)) {
                    let hint = if cfg!(target_os = "macos") { "⌘. to fix" } else { "Ctrl+. to fix" };
                    return Some(div().text_color(theme.muted).whitespace_nowrap().child(hint).into_any_element());
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
                let missing = editor.missing;
                let group = format!("tab-{ix}");
                // Unsaved: a small dot, which turns into the close button under the pointer.
                let close = div()
                    .id(("close", ix))
                    .tooltip(ui::tip("Close", Some(Box::new(CloseTab))))
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
                    .active(|s| s.opacity(0.7))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.close_tab_at(ix, window, cx);
                    }));
                let (name, folder) = labels[ix].clone();
                let dragged = DraggedTab { editor: tab.editor.clone(), label: name.clone().into() };
                div()
                    .id(("tab", ix))
                    .debug_selector(|| format!("tab {name}"))
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
                            // Deleted on disk: struck through, until saved back.
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .when(missing, |d| d.line_through().text_color(theme.muted))
                                    .child(name),
                            )
                            .children(folder.map(|f| div().flex_none().text_color(theme.faint).child(f))),
                    )
                    .child(close)
                    .active(|s| s.opacity(0.7))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.activate(ix, window, cx)))
                    // Right-click: what can be done with the tab.
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            if let Some(tab) = this.tabs.get(ix) {
                                this.tab_menu = Some(TabMenu { editor: tab.editor.clone(), position: event.position });
                                // The keyboard stays with the editor, not the window behind it.
                                window.prevent_default();
                                cx.notify();
                            }
                        }),
                    )
                    // Middle-click closes, as in browsers.
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| this.close_tab_at(ix, window, cx)),
                    )
            }))
            .into_any_element()
    }

    /// The items a tab's menu shows: closing, its path, the other side.
    fn tab_menu_items(&self, editor: &Entity<Editor>, cx: &App) -> Vec<TabMenuItem> {
        use TabMenuItem::*;
        let Some(ix) = self.tabs.iter().position(|t| &t.editor == editor) else { return Vec::new() };
        let mut items = vec![Close];
        if self.tabs.len() > 1 {
            items.push(CloseOthers);
        }
        let side = self.side_tabs(self.tabs[ix].side);
        if side.last() != Some(&ix) {
            items.push(CloseToTheRight);
        }
        if editor.read(cx).path().is_some() {
            items.extend([CopyPath, CopyRelativePath, Reveal, OtherSide]);
            if self.branch.is_some() {
                items.extend([History, CopyLink]);
            }
        }
        items
    }

    fn run_tab_menu_item(&mut self, item: TabMenuItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.tab_menu.take() else { return };
        let editor = menu.editor;
        let Some(ix) = self.tabs.iter().position(|t| t.editor == editor) else { return };
        let path = editor.read(cx).path().map(Path::to_path_buf);
        match item {
            TabMenuItem::Close => self.close_tab_at(ix, window, cx),
            TabMenuItem::CloseOthers => {
                let others = self.tabs.iter().map(|t| t.editor.clone()).filter(|e| *e != editor).collect();
                self.confirm_unsaved(CloseAction::CloseTabs(others), window, cx);
            }
            TabMenuItem::CloseToTheRight => {
                let side = self.side_tabs(self.tabs[ix].side);
                let after = side.iter().skip_while(|&&i| i != ix).skip(1);
                let editors = after.map(|&i| self.tabs[i].editor.clone()).collect();
                self.confirm_unsaved(CloseAction::CloseTabs(editors), window, cx);
            }
            TabMenuItem::CopyPath => self.copy_path(path.as_deref(), false, cx),
            TabMenuItem::CopyRelativePath => self.copy_path(path.as_deref(), true, cx),
            TabMenuItem::Reveal => {
                if let Some(path) = path {
                    cx.reveal_path(&path);
                }
            }
            TabMenuItem::History => {
                self.activate(ix, window, cx);
                self.file_history(&FileHistory, window, cx);
            }
            TabMenuItem::CopyLink => {
                self.activate(ix, window, cx);
                self.copy_line_link(cx);
            }
            TabMenuItem::OtherSide => {
                self.activate(ix, window, cx);
                self.open_on_other_side(window, cx);
            }
        }
        cx.notify();
    }

    /// Files dropped on a Markdown file: a link to each where they landed. One from outside
    /// the project is copied next to the Markdown file first, so the link keeps working.
    fn link_dropped(&mut self, editor: Entity<Editor>, paths: &[PathBuf], at: usize, cx: &mut Context<Self>) {
        let Some(dir) = editor.read(cx).path().and_then(Path::parent).map(Path::to_path_buf) else { return };
        let root = self.tree.read(cx).root().to_path_buf();
        let mut links = Vec::new();
        for path in paths {
            let target = if path.starts_with(&root) {
                path.clone()
            } else {
                match crate::fs_ops::copy_into(path, &dir) {
                    Ok(copy) => copy,
                    Err(problem) => {
                        self.show_notice(problem, cx);
                        continue;
                    }
                }
            };
            links.push(crate::markdown_view::link_to(&dir, &target));
        }
        if !links.is_empty() {
            editor.update(cx, |editor, cx| {
                let ending = editor.style.line_ending.text();
                editor.insert_at(at, &links.join(ending), cx)
            });
        }
    }

    /// A file's path on the clipboard: in full, or from the project's folder.
    fn copy_path(&self, path: Option<&Path>, relative: bool, cx: &mut Context<Self>) {
        let Some(path) = path else { return };
        let root = self.tree.read(cx).root().to_path_buf();
        let shown = if relative { path.strip_prefix(&root).unwrap_or(path) } else { path };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(shown.display().to_string()));
    }

    /// Rename File… and Move File to Trash… from ⌘K: the open file chosen in the files,
    /// which then asks for its new name, or whether to trash it, as it does from there.
    fn open_file_in_tree(&mut self, rename: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.active_path(cx) else { return };
        if !path.starts_with(self.tree.read(cx).root()) {
            return self.show_notice("Only files in the project can be renamed or trashed here.".into(), cx);
        }
        self.show_files(&ShowFiles, window, cx);
        self.tree.update(cx, |tree, cx| {
            tree.set_active(Some(path), cx);
            if rename {
                tree.rename(&crate::file_tree::Rename, window, cx);
            } else {
                tree.trash(&crate::file_tree::Trash, window, cx);
            }
        });
    }

    /// The open file's path, for the commands about it.
    fn active_path(&self, cx: &App) -> Option<PathBuf> {
        self.active_editor()?.read(cx).path().map(Path::to_path_buf)
    }

    fn render_tab_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.tab_menu.as_ref()?;
        let theme = cx.global::<Theme>();
        let items = self.tab_menu_items(&menu.editor, cx).into_iter().enumerate().map(|(i, item)| {
            div()
                .when(item.starts_group() && i > 0, |d| {
                    d.mt(px(4.)).pt(px(4.)).border_t_1().border_color(theme.hairline)
                })
                .child(
                    div()
                        .id(("tab-menu", i))
                        .debug_selector(|| format!("tab-menu {}", item.label()))
                        .h(px(26.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded(px(ui::R_ROW))
                        .cursor_pointer()
                        .text_color(theme.foreground)
                        .hover(|s| s.bg(theme.accent_soft))
                        .child(item.label())
                        .active(|s| s.opacity(0.7))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.run_tab_menu_item(item, window, cx)
                        })),
                )
        });
        Some(
            gpui::deferred(
                gpui::anchored().position(menu.position).snap_to_window_with_margin(px(8.)).child(
                    div()
                        .occlude()
                        .min_w(px(220.))
                        .p(px(4.))
                        .rounded(px(ui::R_POPOVER))
                        .bg(theme.raised)
                        .border_1()
                        .border_color(theme.hairline)
                        .shadow_lg()
                        .text_size(px(ui::T_MD))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.tab_menu = None;
                            cx.notify();
                        }))
                        .children(items),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
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
                    .children(hint(&OpenRecent, "Open a recent project"))
                    .children(hint(&TogglePalette, "Find a file"))
                    .children(hint(&ToggleSidebar, "Show or hide the files")),
            )
            .into_any_element()
    }
}

/// The action that switches to `theme` (menus and ⌘K).
pub fn spacing_action(spacing: crate::settings::LineSpacing) -> Box<dyn Action> {
    use crate::settings::LineSpacing;
    match spacing {
        LineSpacing::Compact => Box::new(CompactLineSpacing),
        LineSpacing::Normal => Box::new(NormalLineSpacing),
        LineSpacing::Relaxed => Box::new(RelaxedLineSpacing),
    }
}

pub fn theme_action(theme: ThemeName) -> Box<dyn Action> {
    match theme {
        ThemeName::Null => Box::new(UseNullTheme),
        ThemeName::Ash => Box::new(UseAshTheme),
        ThemeName::Midnight => Box::new(UseMidnightTheme),
        ThemeName::Moss => Box::new(UseMossTheme),
        ThemeName::Paper => Box::new(UsePaperTheme),
        ThemeName::Dune => Box::new(UseDuneTheme),
    }
}

/// The status bar's path is cut to about this many characters, from the left.
const STATUS_PATH_CHARS: usize = 60;

/// Where the caret is, and how much is selected: "Ln 4, Col 9 · 12 selected" on one line,
/// "· 3 lines selected" over several.
fn position_label(editor: &Editor, line: usize, col: usize) -> String {
    let place = format!("Ln {}, Col {}", line + 1, col + 1);
    let range = editor.selection.range();
    // Prose: how many words, in the file or the selection.
    if editor.is_prose() {
        let plural = |n: usize| if n == 1 { "1 word".to_string() } else { format!("{} words", thousands(n)) };
        return match (range.is_empty(), editor.word_count()) {
            (true, Some(words)) => format!("{place} · {}", plural(words)),
            (true, None) => place,
            (false, _) => {
                format!("{place} · {} selected", plural(crate::editor::words_in(&editor.buffer.slice(range))))
            }
        };
    }
    if range.is_empty() {
        return place;
    }
    let (first, _) = editor.buffer.point(range.start);
    let (last, last_col) = editor.buffer.point(range.end);
    // A selection ending at the start of a line doesn't take that line.
    let lines = last + 1 - first - usize::from(last > first && last_col == 0);
    if lines > 1 {
        format!("{place} · {lines} lines selected")
    } else {
        format!("{place} · {} selected", range.len())
    }
}

/// How much of the changes AI reads to write a commit message, in characters.
const COMMIT_DIFF_CHARS: usize = 24_000;

/// What changed in files (name, committed text, text now), as a unified diff, cut short
/// past `max` characters.
fn changes_as_diff(changes: &[(String, Option<String>, Option<String>)], max: usize) -> String {
    let mut out = String::new();
    for (name, before, after) in changes {
        let (before, after) = (before.as_deref().unwrap_or(""), after.as_deref().unwrap_or(""));
        let diff = similar::TextDiff::from_lines(before, after);
        let patch =
            diff.unified_diff().context_radius(3).header(&format!("a/{name}"), &format!("b/{name}")).to_string();
        out.push_str(&patch);
        if out.len() > max {
            let mut cut = max;
            while !out.is_char_boundary(cut) {
                cut -= 1;
            }
            out.truncate(cut);
            out.push_str("\n[… more changes left out]\n");
            break;
        }
    }
    out
}

/// Lines of code as Markdown: "`src/a.rs` lines 3–5", then the code fenced (named by the
/// file's extension, the fence longer than any in the code), the indentation they share off.
fn code_block(name: &str, ext: &str, first: usize, last: usize, lines: &[String]) -> String {
    let indent = |l: &String| l.len() - l.trim_start_matches([' ', '\t']).len();
    let shared = lines.iter().filter(|l| !l.trim().is_empty()).map(indent).min().unwrap_or(0);
    let body: Vec<&str> = lines.iter().map(|l| l.get(shared..).unwrap_or("").trim_end()).collect();
    let longest_run = body.iter().flat_map(|l| l.split(|c| c != '`').map(str::len)).max().unwrap_or(0);
    let fence = "`".repeat(longest_run.max(2) + 1);
    let place = if first == last { format!("line {first}") } else { format!("lines {first}–{last}") };
    format!("`{name}` {place}\n\n{fence}{ext}\n{}\n{fence}\n", body.join("\n"))
}

/// 1234567 as "1,234,567".
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(d);
    }
    out
}

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
/// Where `path`, gone from disk, went among the `changed` paths: a file holding the same
/// text, alone of its kind. As (what moved, where to): the file itself, or its folder when
/// the folder moved. Files already open elsewhere don't count.
fn moved_to(
    path: &Path,
    print: crate::editor::Fingerprint,
    changed: &[PathBuf],
    open: &[PathBuf],
) -> Option<(PathBuf, PathBuf)> {
    // (what moved, where to, the file it is now), folders first so they win.
    let mut found: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new();
    for gone in changed.iter().filter(|g| path.starts_with(g) && g.as_path() != path && !g.exists()) {
        let Ok(rest) = path.strip_prefix(gone) else { continue };
        found.extend(changed.iter().filter(|v| v.is_dir()).map(|v| (gone.clone(), v.clone(), v.join(rest))));
    }
    found.extend(changed.iter().filter(|v| v.is_file()).map(|v| (path.to_path_buf(), v.clone(), v.clone())));
    let same_text = |file: &Path| {
        file != path
            && !open.iter().any(|o| o == file)
            // Its size, near enough (another encoding writes the same text in other sizes).
            && std::fs::metadata(file).is_ok_and(|m| {
                let len = m.len() as usize;
                m.is_file() && len >= print.0 / 3 && len <= print.0 * 2 + 3
            })
            && crate::encoding::read(file).is_ok_and(|(text, _)| crate::editor::fingerprint(&text) == print)
    };
    found.retain(|(_, _, file)| same_text(file));
    let mut files: Vec<&PathBuf> = found.iter().map(|(_, _, f)| f).collect();
    files.sort();
    files.dedup();
    (files.len() == 1).then(|| found.swap_remove(0)).map(|(from, to, _)| (from, to))
}

/// A version in a file's history: a commit, or one Null wrote over.
#[derive(Clone)]
enum Past {
    Commit(git::FileCommit),
    Saved(crate::local_history::Saved),
}

impl Past {
    /// Unix seconds.
    fn time(&self) -> i64 {
        match self {
            Past::Commit(c) => c.time,
            Past::Saved(s) => s.time / 1000,
        }
    }
}

/// Whether two paths name the same file, one maybe through a link (/tmp and /private/tmp
/// on macOS, as git gives paths). The file itself needn't exist anymore.
fn same_file(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| match (p.parent().and_then(|d| std::fs::canonicalize(d).ok()), p.file_name()) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => p.to_path_buf(),
    };
    a == b || (a.file_name() == b.file_name() && real(a) == real(b))
}

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
        // Focus mode hides the sidebar without changing the setting.
        let sidebar_width = if self.focus_mode { 0. } else { full_width * sidebar };
        let switch = self.render_sidebar_switch(cx);
        let sidebar_content = if self.sidebar_search.on {
            div().flex_1().min_h_0().pt(px(6.)).child(self.project_search.clone())
        } else {
            div().flex_1().min_h_0().child(self.tree.clone())
        };

        let root = self.tree.read(cx).root().to_path_buf();
        let lsp_status = self.language_status(cx);
        let conflicts = self.active_editor().map_or(0, |e| e.read(cx).conflicts().len());
        let (status_items, problems): (Vec<String>, (usize, usize)) = match self.active_editor().map(|e| e.read(cx)) {
            Some(editor) => {
                let (line, col) = editor.caret_point();
                let path = editor.path().map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string());
                let problems = editor.problems(cx);
                let count = |s| problems.iter().filter(|p| p.severity == s).count();
                let mut path = path.map(|p| shorten_path(&p, STATUS_PATH_CHARS)).unwrap_or_else(|| "Untitled".into());
                if editor.missing {
                    path.push_str(" · deleted on disk");
                }
                if let Some(preview) = &editor.preview {
                    (vec![path, preview.summary()], (0, 0))
                } else {
                    (
                        vec![
                            path,
                            match editor.extra.len() {
                                _ if editor.reading => "Preview".into(),
                                0 => position_label(editor, line, col),
                                n => format!("{} cursors · Esc for one", n + 1),
                            },
                            editor.language_name().into(),
                        ],
                        (count(lsp_types::DiagnosticSeverity::ERROR), count(lsp_types::DiagnosticSeverity::WARNING)),
                    )
                }
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
        // While debugging (and after, until closed), the Debug panel takes the terminal's place.
        let debug_panel = (!self.focus_mode).then(|| self.render_debug_panel(cx)).flatten();
        let terminal_panel = match debug_panel {
            Some(panel) => Some(panel),
            None => {
                (!self.focus_mode).then(|| self.render_terminal_panel(TERMINAL_HEIGHT * terminal_shown, cx)).flatten()
            }
        };
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
        let body = if self.focus_mode {
            // Only the side being worked in, its text in a column down the middle.
            let side = self.focused_side();
            let font_size = cx.global::<Settings>().font_size;
            let column = font_size * 0.62 * FOCUS_COLUMNS + 90.;
            div()
                .size_full()
                .flex()
                .justify_center()
                .child(div().h_full().w_full().max_w(px(column)).pt(px(24.)).child(pane(self, side, cx)))
        } else if split {
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
        let tab_menu = self.render_tab_menu(cx);
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
        let encoding =
            self.active_editor().map(|e| e.read(cx).encoding).filter(|e| *e != crate::encoding::Encoding::Utf8);
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
        let status = div()
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
                // The branch (a click switches it), and how many files changed (a click lists them).
                let changed = self.git_status.len();
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(5.))
                    .child(
                        div()
                            .id("branch")
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme.foreground))
                            .tooltip(ui::tip("Switch branch", Some(Box::new(SwitchBranch))))
                            .child(svg().path("icons/branch.svg").size(px(13.)).text_color(theme.muted))
                            .child(branch)
                            .active(|s| s.opacity(0.7))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.switch_branch(&SwitchBranch, window, cx)
                            })),
                    )
                    // Commits to push and to pull, when there are: a click does it.
                    .children(self.sync.filter(|&(ahead, _)| ahead > 0).map(|(ahead, _)| {
                        div()
                            .id("to-push")
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme.foreground))
                            .tooltip(ui::tip(
                                if ahead == 1 {
                                    "1 commit to push".to_string()
                                } else {
                                    format!("{ahead} commits to push")
                                },
                                Some(Box::new(PushBranch)),
                            ))
                            .child(format!("↑{ahead}"))
                            .active(|s| s.opacity(0.7))
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.push_branch(&PushBranch, window, cx)
                                }),
                            )
                    }))
                    .children(self.sync.filter(|&(_, behind)| behind > 0).map(|(_, behind)| {
                        div()
                            .id("to-pull")
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme.foreground))
                            .tooltip(ui::tip(
                                if behind == 1 {
                                    "1 commit to pull".to_string()
                                } else {
                                    format!("{behind} commits to pull")
                                },
                                Some(Box::new(PullBranch)),
                            ))
                            .child(format!("↓{behind}"))
                            .active(|s| s.opacity(0.7))
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.pull_branch(&PullBranch, window, cx)
                                }),
                            )
                    }))
                    .when(changed > 0, |d| {
                        d.child(
                            div()
                                .id("changes")
                                .cursor_pointer()
                                .text_color(theme.git_modified)
                                .hover(|s| s.text_color(theme.foreground))
                                .tooltip(ui::tip("Review changes", Some(Box::new(ReviewChanges))))
                                .child(format!("· {changed}"))
                                .active(|s| s.opacity(0.7))
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.review_changes(&ReviewChanges, window, cx)
                                })),
                        )
                    })
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
                    .active(|s| s.opacity(0.7))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.show_commands_for("indent with", window, cx)
                        }),
                    )
            }))
            // Not UTF-8: which encoding it's kept in; a click offers UTF-8.
            .children(encoding.map(|encoding| {
                div()
                    .id("status-encoding")
                    .flex_none()
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme.foreground))
                    .child(encoding.label())
                    .active(|s| s.opacity(0.7))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.show_commands_for("encoding", window, cx)),
                    )
            }))
            .when(crlf, |bar| {
                bar.child(
                    div()
                        .id("status-crlf")
                        .flex_none()
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme.foreground))
                        .child("CRLF")
                        .active(|s| s.opacity(0.7))
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
                                .active(|s| s.opacity(0.7))
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.stop_ai_task(&StopAiTask, window, cx)
                                })),
                        )
                    })
                    .when(reviewing, |d| {
                        d.cursor_pointer().active(|s| s.opacity(0.7)).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| this.review_ai_task(&ReviewAiTask, window, cx),
                        ))
                    })
            }))
            // AI's only lasting mark: a dot. Its name shows on hover; a click opens its settings.
            .when(ai_provider != ProviderId::Off, |bar| {
                let dot = theme.caret.opacity(0.6);
                let lit = theme.caret;
                bar.child(
                    div()
                        .id("ai-status")
                        .flex_none()
                        .size(px(14.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .group("ai-status")
                        .child(div().size(px(6.)).rounded_full().bg(dot).group_hover("ai-status", move |s| s.bg(lit)))
                        .tooltip(ui::tip(format!("AI · {}", ai_provider.label()), None))
                        .active(|s| s.opacity(0.7))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.open_settings_at(Some(Section::Ai), window, cx)
                        })),
                )
            })
            // Merge conflicts left in the file: a click goes to the next.
            .when(conflicts > 0, |bar| {
                bar.child(
                    div()
                        .id("conflicts")
                        .flex_none()
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .text_color(theme.error)
                        .child(if conflicts == 1 { "1 conflict".to_string() } else { format!("{conflicts} conflicts") })
                        .active(|s| s.opacity(0.7))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            if let Some(editor) = this.active_editor().cloned() {
                                editor.update(cx, |e, cx| e.go_to_conflict(true, cx));
                                window.focus(&editor.focus_handle(cx));
                            }
                        })),
                )
            })
            .when(problems != (0, 0), |bar| {
                let (errors, warnings) = problems;
                let plural = |n: usize, word: &str| if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") };
                bar.child(
                    div()
                        .id("problems")
                        .flex()
                        .flex_none()
                        .whitespace_nowrap()
                        .gap(px(10.))
                        .cursor_pointer()
                        .active(|s| s.opacity(0.7))
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.show_problems(&ShowProblems, window, cx)
                            }),
                        )
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
            // Files dropped from the Finder open, as with Open With (a folder becomes the project).
            .on_drop(cx.listener(|this, dropped: &gpui::ExternalPaths, window, cx| {
                this.open_paths(dropped.paths().to_vec(), window, cx)
            }))
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
            .on_action(cx.listener(Self::review_changes))
            .on_action(cx.listener(Self::commit_all))
            .on_action(cx.listener(Self::push_branch))
            .on_action(cx.listener(Self::pull_branch))
            .on_action(cx.listener(Self::file_history))
            .on_action(cx.listener(|this, _: &CompareWithSaved, window, cx| this.compare_with_saved(window, cx)))
            .on_action(cx.listener(|this, _: &CopyLineLink, _, cx| this.copy_line_link(cx)))
            .on_action(cx.listener(|this, _: &OpenLineOnWeb, _, cx| this.line_link(true, cx)))
            .on_action(cx.listener(Self::fetch_branch))
            .on_action(cx.listener(|this, _: &CopyAsCodeBlock, _, cx| this.copy_as_code_block(cx)))
            .on_action(
                cx.listener(|this, _: &CopyFilePath, _, cx| this.copy_path(this.active_path(cx).as_deref(), false, cx)),
            )
            .on_action(cx.listener(|this, _: &CopyRelativeFilePath, _, cx| {
                this.copy_path(this.active_path(cx).as_deref(), true, cx)
            }))
            .on_action(cx.listener(|this, _: &RenameFile, window, cx| this.open_file_in_tree(true, window, cx)))
            .on_action(cx.listener(|this, _: &TrashFile, window, cx| this.open_file_in_tree(false, window, cx)))
            .on_action(cx.listener(|this, _: &RevealFile, _, cx| {
                if let Some(path) = this.active_path(cx) {
                    cx.reveal_path(&path);
                }
            }))
            .on_action(
                cx.listener(|this, _: &CompareWithClipboard, window, cx| this.compare_with_clipboard(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CompareWithFile, window, cx| this.pick_file_to_compare(window, cx)))
            .on_action(cx.listener(Self::switch_branch))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_to_last_edit))
            .on_action(cx.listener(Self::go_forward))
            // The mouse's back and forward buttons.
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Back),
                cx.listener(|this, _: &MouseDownEvent, window, cx| this.navigate(true, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Forward),
                cx.listener(|this, _: &MouseDownEvent, window, cx| this.navigate(false, window, cx)),
            )
            .on_action(cx.listener(Self::revert_all_changes))
            .on_action(cx.listener(Self::undo_last_commit))
            .on_action(cx.listener(Self::set_changes_aside))
            .on_action(cx.listener(Self::find_todos))
            .on_action(cx.listener(|this, _: &OpenInBrowser, _, cx| {
                // Saved first: the browser reads the file.
                if let Some(editor) = this.active_editor().cloned()
                    && let Some(path) = editor.read(cx).path().map(Path::to_path_buf)
                    && crate::file_tree::opens_in_browser(&path)
                {
                    Self::save_if_named(&editor, cx);
                    crate::file_tree::open_in_browser(&path, cx);
                }
            }))
            .on_action(cx.listener(Self::bring_back_changes))
            .on_action(cx.listener(|this, _: &ShowWelcome, window, cx| this.show_welcome(window, cx)))
            .on_action(cx.listener(Self::install_shell_command))
            .on_action(cx.listener(Self::review_ai_task))
            .on_action(cx.listener(Self::stop_ai_task))
            .on_action(cx.listener(Self::keep_all_task_changes))
            .on_action(cx.listener(Self::undo_all_task_changes))
            .on_action(cx.listener(|this, _: &MoveTabRight, window, cx| this.move_tab_to(1, window, cx)))
            .on_action(cx.listener(|this, _: &MoveTabLeft, window, cx| this.move_tab_to(0, window, cx)))
            .on_action(cx.listener(|this, _: &OpenOnOtherSide, window, cx| this.open_on_other_side(window, cx)))
            .on_action(cx.listener(Self::go_to_symbol))
            .on_action(cx.listener(Self::go_to_symbol_in_project))
            .on_action(cx.listener(|_, _: &ToggleFormatOnSave, _, cx| {
                settings::update(cx, |s| s.format_on_save = !s.format_on_save)
            }))
            .on_action(cx.listener(|this, _: &AutoSaveOff, _, cx| {
                this.auto_saves.clear();
                settings::update(cx, |s| s.auto_save = AutoSave::Off)
            }))
            .on_action(
                cx.listener(|_, _: &ToggleLineBlame, _, cx| settings::update(cx, |s| s.line_blame = !s.line_blame)),
            )
            .on_action(
                cx.listener(|_, _: &ToggleSpellCheck, _, cx| settings::update(cx, |s| s.spell_check = !s.spell_check)),
            )
            .on_action(
                cx.listener(|_, _: &ToggleInlayHints, _, cx| settings::update(cx, |s| s.inlay_hints = !s.inlay_hints)),
            )
            .on_action(cx.listener(Self::toggle_focus_mode))
            .on_action(cx.listener(Self::run_task))
            .on_action(cx.listener(|this, _: &RunTestAtCursor, window, cx| this.run_test(true, window, cx)))
            .on_action(cx.listener(Self::open_preview_to_the_side))
            .on_action(cx.listener(|this, _: &RunTestsInFile, window, cx| this.run_test(false, window, cx)))
            .on_action(cx.listener(Self::open_recent))
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(Self::start_debugging))
            .on_action(cx.listener(|this, _: &StopDebugging, _, cx| {
                this.debugger.update(cx, |d, cx| d.stop(cx));
            }))
            .on_action(cx.listener(|this, _: &PauseDebugging, _, cx| this.debugger.update(cx, |d, _| d.pause())))
            .on_action(cx.listener(|this, _: &StepOver, _, cx| this.debugger.update(cx, |d, cx| d.resume("next", cx))))
            .on_action(
                cx.listener(|this, _: &StepInto, _, cx| this.debugger.update(cx, |d, cx| d.resume("stepIn", cx))),
            )
            .on_action(
                cx.listener(|this, _: &StepOut, _, cx| this.debugger.update(cx, |d, cx| d.resume("stepOut", cx))),
            )
            .on_action(cx.listener(Self::new_terminal))
            .on_action(cx.listener(Self::next_terminal))
            .on_action(cx.listener(Self::next_problem))
            .on_action(
                cx.listener(|_, _: &ToggleSymbolMarks, _, cx| {
                    settings::update(cx, |s| s.symbol_marks = !s.symbol_marks)
                }),
            )
            .on_action(cx.listener(|_, _: &ToggleStickyScroll, _, cx| {
                settings::update(cx, |s| s.sticky_scroll = !s.sticky_scroll)
            }))
            .on_action(cx.listener(|_, _: &ToggleIndentGuides, _, cx| {
                settings::update(cx, |s| s.indent_guides = !s.indent_guides)
            }))
            .on_action(
                cx.listener(|_, _: &ToggleLineGuide, _, cx| settings::update(cx, |s| s.line_guide = !s.line_guide)),
            )
            .on_action(cx.listener(Self::previous_problem))
            .on_action(cx.listener(|_, _: &AutoSaveAfterPause, _, cx| {
                settings::update(cx, |s| s.auto_save = AutoSave::AfterPause)
            }))
            .on_action(cx.listener(|_, _: &AutoSaveWhenLeaving, _, cx| {
                settings::update(cx, |s| s.auto_save = AutoSave::WhenLeaving)
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
            .on_action(cx.listener(|_, _: &UseNullTheme, _, cx| settings::update(cx, |s| s.theme = ThemeName::Null)))
            .on_action(cx.listener(|_, _: &CompactLineSpacing, _, cx| {
                settings::update(cx, |s| s.line_spacing = crate::settings::LineSpacing::Compact)
            }))
            .on_action(cx.listener(|_, _: &NormalLineSpacing, _, cx| {
                settings::update(cx, |s| s.line_spacing = crate::settings::LineSpacing::Normal)
            }))
            .on_action(cx.listener(|_, _: &RelaxedLineSpacing, _, cx| {
                settings::update(cx, |s| s.line_spacing = crate::settings::LineSpacing::Relaxed)
            }))
            .on_action(cx.listener(|_, _: &UseAshTheme, _, cx| settings::update(cx, |s| s.theme = ThemeName::Ash)))
            .on_action(
                cx.listener(|_, _: &UseMidnightTheme, _, cx| settings::update(cx, |s| s.theme = ThemeName::Midnight)),
            )
            .on_action(cx.listener(|_, _: &UseMossTheme, _, cx| settings::update(cx, |s| s.theme = ThemeName::Moss)))
            .on_action(cx.listener(|_, _: &UsePaperTheme, _, cx| settings::update(cx, |s| s.theme = ThemeName::Paper)))
            .on_action(cx.listener(|_, _: &UseDuneTheme, _, cx| settings::update(cx, |s| s.theme = ThemeName::Dune)))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::search_project))
            .on_action(cx.listener(Self::show_files))
            .on_action(cx.listener(Self::quit))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            // In Focus mode, only a strip to drag the window by.
            .map(|root| {
                if self.focus_mode {
                    root.child(div().h(px(40.)).flex_none().window_control_area(WindowControlArea::Drag))
                } else {
                    root.child(titlebar)
                }
            })
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
            .when(!self.focus_mode, |root| root.child(status))
            .children(tab_menu)
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

    #[test]
    fn a_moved_file_is_found_by_its_text() {
        let dir = std::env::temp_dir().join(format!("null-moves-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("new")).unwrap();
        let print = crate::editor::fingerprint("fn a() {}\n");
        std::fs::write(dir.join("b.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.join("other.rs"), "fn other() {}\n").unwrap();
        // a.rs renamed to b.rs, in the same burst as another file.
        let changed = [dir.join("a.rs"), dir.join("b.rs"), dir.join("other.rs")];
        assert_eq!(moved_to(&dir.join("a.rs"), print, &changed, &[]), Some((dir.join("a.rs"), dir.join("b.rs"))));
        // Already open elsewhere: not a move.
        assert_eq!(moved_to(&dir.join("a.rs"), print, &changed, &[dir.join("b.rs")]), None);
        // Two files with the same text: can't tell which, so neither.
        std::fs::write(dir.join("c.rs"), "fn a() {}\n").unwrap();
        assert_eq!(moved_to(&dir.join("a.rs"), print, &[changed.to_vec(), vec![dir.join("c.rs")]].concat(), &[]), None);
        // Its folder moved: the folder is what moved, so the others in it follow too.
        std::fs::write(dir.join("new/a.rs"), "fn a() {}\n").unwrap();
        let changed = [dir.join("old"), dir.join("new"), dir.join("new/a.rs")];
        assert_eq!(moved_to(&dir.join("old/a.rs"), print, &changed, &[]), Some((dir.join("old"), dir.join("new"))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn replacing_across_files_keeps_each_file_s_encoding(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-replace-enc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("latin.txt"), b"caf\xe9 noir\n").unwrap();
        std::fs::write(dir.join("utf8.txt"), "café noir\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update(cx, |w, cx| {
            let query = crate::search::SearchQuery { text: "noir".into(), ..Default::default() };
            let targets = [(dir.join("latin.txt"), None), (dir.join("utf8.txt"), None)];
            w.replace_in_files(&query, "crème", &targets, cx);
            // Something Windows-1252 can't hold: that file is left as it was.
            let query = crate::search::SearchQuery { text: "crème".into(), ..Default::default() };
            w.replace_in_files(&query, "crème 😀", &targets[..1], cx);
        });
        assert_eq!(std::fs::read(dir.join("latin.txt")).unwrap(), b"caf\xe9 cr\xe8me\n");
        assert_eq!(std::fs::read_to_string(dir.join("utf8.txt")).unwrap(), "café crème\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The git lists, as a person goes through them: Review Changes and ↵ opens a file's
    /// changes, a file's history and ↵ on a commit compares with it, Commit with a file
    /// left out (⇥) commits the rest.
    #[gpui::test]
    #[cfg(unix)]
    fn git_lists_do_what_their_rows_say(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-git-lists-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().map(|o| o.status.success())
        };
        if !git(&["init", "-q"]).unwrap_or(false) {
            return; // No git here.
        }
        git(&["config", "user.name", "t"]).unwrap();
        git(&["config", "user.email", "t@t"]).unwrap();
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        git(&["add", "-A"]).unwrap();
        git(&["commit", "-qm", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        git(&["commit", "-qam", "second"]).unwrap();
        // Now changed: a.txt (a line added), b.txt.
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(dir.join("b.txt"), "b2\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let settle = |cx: &mut gpui::VisualTestContext| {
            cx.executor().advance_clock(Duration::from_millis(500));
            cx.run_until_parked();
        };
        settle(cx);
        assert_eq!(workspace.read_with(cx, |w, _| w.git_status.len()), 2);

        // Review Changes, ↵ on its first row: that file opens with its changes to review.
        workspace.update_in(cx, |w, window, cx| w.review_changes(&ReviewChanges, window, cx));
        settle(cx);
        cx.simulate_keystrokes("enter");
        settle(cx);
        workspace.update(cx, |w, cx| {
            let editor = w.active_editor().unwrap().read(cx);
            assert_eq!(editor.file_name(), "a.txt");
            assert!(editor.in_review(), "the file opened without its changes");
        });

        // a.txt's history, ↵ on the older commit: compared with "one".
        workspace.update_in(cx, |w, window, cx| w.file_history(&FileHistory, window, cx));
        settle(cx);
        cx.simulate_keystrokes("down enter");
        settle(cx);
        workspace.update(cx, |w, cx| {
            let editor = w.active_editor().unwrap().read(cx);
            assert!(editor.in_review());
            assert_eq!(editor.review_base(), Some("one\n"));
        });

        // Commit, with the first file (a.txt) left out: only b.txt is committed.
        workspace.update_in(cx, |w, window, cx| w.commit_all(&CommitAll, window, cx));
        settle(cx);
        cx.simulate_input("only b");
        cx.simulate_keystrokes("tab enter");
        settle(cx);
        let committed = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["show", "--name-only", "--format=%s", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&committed.stdout).split_whitespace().collect::<Vec<_>>(),
            ["only", "b", "b.txt"]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The palettes, by keystroke: ⌘P with a place, ⌘P ":line", ⌃G, ⌘⇧O and a name, ⌘K and
    /// a command. Each row does what it says.
    #[gpui::test]
    fn palette_rows_go_where_they_say(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-palette-flows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "fn alpha() {}\n\nfn beta() {\n    alpha();\n}\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let settle = |cx: &mut gpui::VisualTestContext| {
            cx.executor().advance_clock(Duration::from_millis(500));
            cx.run_until_parked();
        };
        // As when Null starts: the window's keys go to the workspace.
        workspace.update_in(cx, |w, window, cx| window.focus(&w.focus_handle(cx)));
        settle(cx);
        let place = |cx: &mut gpui::VisualTestContext| {
            workspace.read_with(cx, |w, cx| {
                let e = w.active_editor().map(|e| e.read(cx));
                e.map(|e| (e.file_name(), e.caret_point()))
            })
        };
        // ⌘P "notes.txt:3:2": that file, line 3, column 2.
        cx.simulate_keystrokes("cmd-p");
        settle(cx);
        cx.simulate_input("notes.txt:3:2");
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert_eq!(place(cx), Some(("notes.txt".into(), (2, 1))));
        // ⌘P ":2", and ⌃G "4": lines in the open file.
        cx.simulate_keystrokes("cmd-p");
        settle(cx);
        cx.simulate_input(":2");
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert_eq!(place(cx), Some(("notes.txt".into(), (1, 0))));
        cx.simulate_keystrokes("ctrl-g");
        settle(cx);
        cx.simulate_input("4");
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert_eq!(place(cx).map(|(_, (line, _))| line), Some(3));
        // ⌘⇧O in lib.rs, "beta": its definition.
        cx.simulate_keystrokes("cmd-p");
        settle(cx);
        cx.simulate_input("lib.rs");
        cx.simulate_keystrokes("enter");
        settle(cx);
        cx.simulate_keystrokes("cmd-shift-o");
        settle(cx);
        cx.simulate_input("beta");
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert_eq!(place(cx), Some(("lib.rs".into(), (2, 3))));
        // ⌘K "toggle sidebar": the sidebar goes.
        let visible = |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| cx.global::<Settings>().sidebar_visible);
        let before = visible(cx);
        cx.simulate_keystrokes("cmd-k");
        settle(cx);
        cx.simulate_input("toggle sidebar");
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert_ne!(visible(cx), before);
        // A quick setting stays open to see its effect; Escape closes it.
        cx.simulate_keystrokes("escape");
        settle(cx);
        assert!(workspace.read_with(cx, |w, _| w.palette.is_none()));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Finding by keystroke: ⌘F and ↵ step through a file's matches, ⌘⇧F and ↵ opens a
    /// project search's result, ⌘W and ⌘⇧T close and bring back a tab.
    #[gpui::test]
    fn find_search_and_tabs_by_keystroke(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-find-flows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "let alpha = 1;\nlet beta = alpha + alpha;\n").unwrap();
        std::fs::write(dir.join("src/b.rs"), "fn other() {}\n\n// gamma lives here\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let settle = |cx: &mut gpui::VisualTestContext| {
            for _ in 0..4 {
                cx.executor().advance_clock(Duration::from_millis(300));
                cx.run_until_parked();
            }
        };
        workspace.update_in(cx, |w, window, cx| window.focus(&w.focus_handle(cx)));
        settle(cx);
        let place = |cx: &mut gpui::VisualTestContext| {
            workspace
                .read_with(cx, |w, cx| w.active_editor().map(|e| (e.read(cx).file_name(), e.read(cx).caret_point())))
        };
        cx.simulate_keystrokes("cmd-p");
        settle(cx);
        cx.simulate_input("a.rs");
        cx.simulate_keystrokes("enter");
        settle(cx);
        // ⌘F "alpha": the first one; ↵ the next, and the next.
        cx.simulate_keystrokes("cmd-f");
        settle(cx);
        cx.simulate_input("alpha");
        settle(cx);
        let lines: Vec<(usize, usize)> = (0..2)
            .map(|_| {
                cx.simulate_keystrokes("enter");
                settle(cx);
                place(cx).unwrap().1
            })
            .collect();
        assert_eq!(lines, [(1, 16), (1, 24)], "↵ goes from match to match");
        cx.simulate_keystrokes("escape");
        settle(cx);
        // ⌘⇧F "gamma", ↵: b.rs opens on that line.
        cx.simulate_keystrokes("cmd-shift-f");
        settle(cx);
        cx.simulate_input("gamma");
        settle(cx);
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert_eq!(place(cx).map(|(name, (line, _))| (name, line)), Some(("b.rs".into(), 2)));
        // ⌘W closes it; ⌘⇧T brings it back.
        workspace.update_in(cx, |w, window, cx| {
            let editor = w.active_editor().unwrap().focus_handle(cx);
            window.focus(&editor);
        });
        cx.simulate_keystrokes("cmd-w");
        settle(cx);
        assert_eq!(place(cx).map(|(name, _)| name), Some("a.rs".into()));
        cx.simulate_keystrokes("cmd-shift-t");
        settle(cx);
        assert_eq!(place(cx).map(|(name, _)| name), Some("b.rs".into()));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Dragging a file onto a folder in the tree, with the real mouse events: it moves
    /// there, and its tab follows.
    #[gpui::test]
    fn dragging_a_file_onto_a_folder_moves_it(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-tree-drag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("notes.txt"), "hello\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let settle = |cx: &mut gpui::VisualTestContext| {
            for _ in 0..4 {
                cx.executor().advance_clock(Duration::from_millis(600));
                cx.run_until_parked();
            }
        };
        workspace.update_in(cx, |w, window, cx| w.open_file(dir.join("notes.txt"), window, cx));
        settle(cx);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let file = format!("tree-row {}", dir.join("notes.txt").display());
        let folder = format!("tree-row {}", dir.join("src").display());
        let (file, folder): (&'static str, &'static str) = (file.leak(), folder.leak());
        let from = cx.debug_bounds(file).expect("the file's row is drawn").center();
        let to = cx.debug_bounds(folder).expect("the folder's row is drawn").center();
        let none = gpui::Modifiers::default();
        cx.simulate_mouse_down(from, gpui::MouseButton::Left, none);
        for step in 1..=6 {
            let t = step as f32 / 6.;
            let p = gpui::point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            cx.simulate_mouse_move(p, gpui::MouseButton::Left, none);
        }
        cx.simulate_mouse_up(to, gpui::MouseButton::Left, none);
        settle(cx);
        assert!(dir.join("src/notes.txt").is_file() && !dir.join("notes.txt").exists());
        workspace.update(cx, |w, cx| {
            assert_eq!(w.active_editor().unwrap().read(cx).path(), Some(dir.join("src/notes.txt").as_path()));
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Clicks inside the palette and Settings leave the keyboard there: after clicking a
    /// quick setting, typing goes on in the palette; after clicking a switch in Settings,
    /// Escape still closes it.
    #[gpui::test]
    fn clicks_in_the_palette_and_settings_keep_the_keyboard(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-overlay-clicks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| window.focus(&w.focus_handle(cx)));
        cx.run_until_parked();
        // ⌘K, a click on its first quick setting (it stays open), then typing: the query.
        cx.simulate_keystrokes("cmd-k");
        cx.run_until_parked();
        let row = cx.debug_bounds("palette-row 0").expect("a row is drawn").center();
        cx.simulate_click(row, Default::default());
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |w, _| w.palette.is_some()), "a quick setting keeps the palette open");
        cx.simulate_input("wrap");
        cx.run_until_parked();
        let query = workspace.read_with(cx, |w, cx| w.palette.as_ref().map(|(p, _)| p.read(cx).query().to_string()));
        assert_eq!(query.as_deref(), Some("wrap"), "typing after the click goes to the palette");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        // ⌘F in a file, a click on "Match case", then more typing: still the find field's.
        std::fs::write(dir.join("a.txt"), "Alpha alpha\n").unwrap();
        workspace.update_in(cx, |w, window, cx| w.open_file(dir.join("a.txt"), window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-f");
        cx.run_until_parked();
        cx.simulate_input("al");
        cx.run_until_parked();
        let case = cx.debug_bounds("find case").expect("the button is drawn").center();
        cx.simulate_click(case, Default::default());
        cx.run_until_parked();
        cx.simulate_input("pha");
        cx.run_until_parked();
        let (find, text) = workspace.read_with(cx, |w, cx| {
            let e = w.active_editor().unwrap().read(cx);
            (e.find_query(cx), e.buffer.to_string())
        });
        assert_eq!(text, "Alpha alpha\n", "nothing typed into the file");
        let find = find.expect("the find bar is open");
        assert!(find.case_sensitive && find.text == "alpha", "{find:?}");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        // Settings, a click on a switch, then Escape: closed.
        cx.simulate_keystrokes("cmd-,");
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |w, _| w.settings_panel.is_some()), "⌘, opens Settings");
        workspace.update_in(cx, |w, _, cx| {
            if let Some((panel, _)) = &w.settings_panel {
                panel.update(cx, |p, cx| {
                    p.show_section(crate::settings_panel::Section::Editor);
                    cx.notify();
                });
            }
        });
        cx.run_until_parked();
        let wrap = cx.debug_bounds("toggle wrap").expect("the switch is drawn").center();
        let before = cx.update(|_, cx| cx.global::<Settings>().word_wrap);
        cx.simulate_click(wrap, Default::default());
        cx.run_until_parked();
        assert_ne!(cx.update(|_, cx| cx.global::<Settings>().word_wrap), before, "the switch switched");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |w, _| w.settings_panel.is_none()), "Escape closes Settings after a click");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Compare with Saved shows what was typed since saving, to keep or take back;
    /// Compare with Clipboard, how the file differs from the clipboard.
    #[gpui::test]
    fn comparing_with_saved_and_with_the_clipboard(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-compare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| w.open_file(dir.join("a.txt"), window, cx));
        cx.run_until_parked();
        cx.simulate_input("zero\n");
        let text = |cx: &mut gpui::VisualTestContext| {
            workspace.read_with(cx, |w, cx| w.active_editor().unwrap().read(cx).buffer.to_string())
        };
        assert_eq!(text(cx), "zero\none\ntwo\n");
        workspace.update_in(cx, |w, window, cx| w.compare_with_saved(window, cx));
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |w, cx| w.active_editor().unwrap().read(cx).in_review()));
        // Esc takes the change back: as saved.
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(text(cx), "one\ntwo\n");
        // The clipboard has another version: its difference shows, and is taken.
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("one\nTWO\n".into()));
        workspace.update_in(cx, |w, window, cx| w.compare_with_clipboard(window, cx));
        cx.run_until_parked();
        let base =
            workspace.read_with(cx, |w, cx| w.active_editor().unwrap().read(cx).review_base().map(str::to_string));
        assert_eq!(base.as_deref(), Some("one\nTWO\n"));
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(text(cx), "one\nTWO\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Markdown wraps while code doesn't; ⌥Z in each switches its own kind.
    #[gpui::test]
    fn prose_wraps_on_its_own_setting(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-prose-wrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let long = "word ".repeat(200);
        std::fs::write(dir.join("notes.md"), format!("{long}\n")).unwrap();
        std::fs::write(dir.join("main.rs"), format!("// {long}\n")).unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let wraps = |cx: &mut gpui::VisualTestContext, name: &str| {
            let path = dir.join(name);
            workspace.update_in(cx, |w, window, cx| w.open_file(path, window, cx));
            cx.run_until_parked();
            workspace.read_with(cx, |w, cx| w.active_editor().unwrap().read(cx).wrap.is_on())
        };
        assert!(wraps(cx, "notes.md"), "Markdown wraps by default");
        assert!(!wraps(cx, "main.rs"), "code doesn't");
        // ⌥Z in the Markdown file: prose stops wrapping, code is left as it was.
        wraps(cx, "notes.md");
        cx.simulate_keystrokes("alt-z");
        cx.run_until_parked();
        let (prose, code) = cx.update(|_, cx| (cx.global::<Settings>().wrap_prose, cx.global::<Settings>().word_wrap));
        assert_eq!((prose, code), (false, false));
        assert!(!wraps(cx, "notes.md"));
        // ⌥Z in the code: code wraps, prose stays off.
        wraps(cx, "main.rs");
        cx.simulate_keystrokes("alt-z");
        cx.run_until_parked();
        let (prose, code) = cx.update(|_, cx| (cx.global::<Settings>().wrap_prose, cx.global::<Settings>().word_wrap));
        assert_eq!((prose, code), (false, true));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Tests run side by side: two with the same scratch folder overwrite each other's
    /// files (it happened). Every folder name in the tests is its own.
    #[test]
    fn tests_use_folders_of_their_own() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let pattern = regex::Regex::new(r#"temp_dir\(\)\.join\(format!\("(null-[A-Za-z0-9-]+)"#).unwrap();
        let mut names: Vec<String> = Vec::new();
        for entry in
            ignore::Walk::new(&src).filter_map(Result::ok).filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
        {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            names.extend(pattern.captures_iter(&text).map(|c| c[1].to_string()));
        }
        let mut seen = std::collections::HashSet::new();
        let shared: Vec<&String> = names.iter().filter(|n| !seen.insert(*n)).collect();
        assert!(names.len() > 20, "found {} folder names: the pattern no longer matches", names.len());
        assert!(shared.is_empty(), "tests share scratch folders: {shared:?}");
    }

    /// A right-click on a tab, then a pick from its menu: the keyboard is still the
    /// editor's, so typing goes on where it was.
    #[gpui::test]
    fn after_a_tab_s_menu_typing_goes_on_in_the_editor(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-tab-menu-typing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| w.open_file(dir.join("a.txt"), window, cx));
        cx.run_until_parked();
        let tab = cx.debug_bounds("tab a.txt").expect("the tab is drawn").center();
        cx.simulate_event(gpui::MouseDownEvent {
            position: tab,
            button: gpui::MouseButton::Right,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |w, _| w.tab_menu.is_some()), "the menu opened");
        let copy = cx.debug_bounds("tab-menu Copy Path").expect("the menu is drawn").center();
        cx.simulate_click(copy, Default::default());
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |w, _| w.tab_menu.is_none()), "picking an item closes the menu");
        cx.simulate_input("x");
        cx.run_until_parked();
        workspace.read_with(cx, |w, cx| {
            assert_eq!(w.active_editor().unwrap().read(cx).buffer.to_string(), "xa\n");
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Discard Changes… on a file: asked first, then back to its last commit, the open tab too.
    #[gpui::test]
    #[cfg(unix)]
    fn a_files_changes_are_discarded(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-discard-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&dir).args(args).output();
        if git(&["init", "-q", "-b", "main"]).is_err() {
            return; // No git here.
        }
        git(&["config", "user.name", "t"]).unwrap();
        git(&["config", "user.email", "t@t"]).unwrap();
        std::fs::write(dir.join("f.txt"), "one\n").unwrap();
        git(&["add", "-A"]).unwrap();
        git(&["commit", "-qm", "first"]).unwrap();
        std::fs::write(dir.join("f.txt"), "ONE\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let file = dir.join("f.txt");
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(file.clone(), window, cx);
            w.discard_changes(file.clone(), git::FileStatus::Modified, window, cx);
        });
        // Cancel first: nothing changes.
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "ONE\n");
        workspace.update_in(cx, |w, window, cx| w.discard_changes(file.clone(), git::FileStatus::Modified, window, cx));
        cx.simulate_prompt_answer("Discard");
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\n");
        workspace.read_with(cx, |w, cx| assert_eq!(w.active_editor().unwrap().read(cx).buffer.to_string(), "one\n"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Undo Last Commit, then Commit: the message is back in the field, ↵ commits again.
    #[gpui::test]
    #[cfg(unix)]
    fn an_undone_commit_s_message_waits_in_commit(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-undo-commit-flow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git").arg("-C").arg(&dir).args(args).output();
            out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        };
        if git(&["init", "-q"]).is_err() {
            return; // No git here.
        }
        git(&["config", "user.name", "t"]).unwrap();
        git(&["config", "user.email", "t@t"]).unwrap();
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        git(&["add", "-A"]).unwrap();
        git(&["commit", "-qm", "first"]).unwrap();
        std::fs::write(dir.join("a.txt"), "ab\n").unwrap();
        git(&["commit", "-qam", "Add b"]).unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let settle = |cx: &mut gpui::VisualTestContext| {
            cx.executor().advance_clock(Duration::from_millis(500));
            cx.run_until_parked();
        };
        settle(cx);
        workspace.update_in(cx, |w, window, cx| w.undo_last_commit(&UndoLastCommit, window, cx));
        settle(cx);
        assert_eq!(git(&["log", "-1", "--format=%s"]).unwrap(), "first");
        workspace.update_in(cx, |w, window, cx| w.commit_all(&CommitAll, window, cx));
        settle(cx);
        let query = workspace.read_with(cx, |w, cx| w.palette.as_ref().map(|(p, _)| p.read(cx).query().to_string()));
        assert_eq!(query.as_deref(), Some("Add b"));
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert_eq!(git(&["log", "-1", "--format=%s"]).unwrap(), "Add b");
        assert!(workspace.read_with(cx, |w, _| w.commit_draft.is_none()));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// No git: a file's history is what Null wrote over. ↵ on it compares, change by change.
    #[gpui::test]
    fn history_without_git_keeps_what_saves_replaced(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-history-no-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "one\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let settle = |cx: &mut gpui::VisualTestContext| {
            cx.executor().advance_clock(Duration::from_millis(500));
            cx.run_until_parked();
        };
        workspace.update_in(cx, |w, window, cx| w.open_file(file.clone(), window, cx));
        let editor = workspace.read_with(cx, |w, _| w.active_editor().unwrap().clone());
        editor.update(cx, |e, cx| {
            e.restore_unsaved("one\ntwo\n", cx);
            assert!(e.save_to_disk(cx));
        });
        workspace.update_in(cx, |w, window, cx| w.file_history(&FileHistory, window, cx));
        settle(cx);
        cx.simulate_keystrokes("enter");
        settle(cx);
        editor.read_with(cx, |e, _| {
            assert!(e.in_review(), "compared with the version saved over");
            assert_eq!(e.review_base(), Some("one\n"));
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file changed on disk under unsaved edits: saving asks before writing over it,
    /// auto-save leaves it be, and Cancel writes nothing.
    #[gpui::test]
    fn saving_over_a_file_changed_on_disk_asks(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-save-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, "one\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| w.open_file(file.clone(), window, cx));
        let editor = workspace.read_with(cx, |w, _| w.active_editor().unwrap().clone());
        editor.update(cx, |e, cx| e.restore_unsaved("mine\n", cx));
        std::fs::write(&file, "theirs\n").unwrap();
        editor.update(cx, |e, cx| e.reload_from_disk(cx));
        cx.run_until_parked();
        assert!(editor.read_with(cx, |e, _| e.disk_changed));
        // Auto-save doesn't write over it.
        cx.update(|_, cx| Workspace::save_if_named(&editor, cx));
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "theirs\n");
        // A save asks; Cancel keeps theirs on disk and mine in the editor.
        editor.update(cx, |e, cx| e.save_to_disk(cx));
        cx.run_until_parked();
        assert!(cx.has_pending_prompt(), "asked before writing over it");
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "theirs\n");
        assert_eq!(editor.read_with(cx, |e, _| e.buffer.to_string()), "mine\n");
        // Save Mine writes it.
        editor.update(cx, |e, cx| e.save_to_disk(cx));
        cx.run_until_parked();
        cx.simulate_prompt_answer("Save Mine");
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "mine\n");
        assert!(!editor.read_with(cx, |e, _| e.disk_changed));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Switching where changes here would be overwritten: asked, they come along.
    #[gpui::test]
    #[cfg(unix)]
    fn switching_offers_to_bring_changes_along(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-carry-changes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git").arg("-C").arg(&dir).args(args).output();
            out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        };
        if git(&["init", "-q", "-b", "main"]).is_err() {
            return; // No git here.
        }
        git(&["config", "user.name", "t"]).unwrap();
        git(&["config", "user.email", "t@t"]).unwrap();
        std::fs::write(dir.join("f.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        git(&["add", "-A"]).unwrap();
        git(&["commit", "-qm", "first"]).unwrap();
        git(&["switch", "-q", "-c", "other"]).unwrap();
        std::fs::write(dir.join("f.txt"), "ONE\ntwo\nthree\nfour\n").unwrap();
        git(&["commit", "-qam", "other's"]).unwrap();
        git(&["switch", "-q", "main"]).unwrap();
        std::fs::write(dir.join("f.txt"), "one\ntwo\nthree\nFOUR\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let settle = |cx: &mut gpui::VisualTestContext| {
            cx.executor().advance_clock(Duration::from_millis(500));
            cx.run_until_parked();
        };
        settle(cx);
        workspace.update_in(cx, |w, window, cx| w.switch_branch(&SwitchBranch, window, cx));
        settle(cx);
        cx.simulate_input("other");
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert!(cx.has_pending_prompt(), "asked whether to bring the changes along");
        cx.simulate_prompt_answer("Bring Them Along");
        settle(cx);
        assert_eq!(git(&["branch", "--show-current"]).unwrap(), "other");
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "ONE\ntwo\nthree\nFOUR\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Switch Branch: typing part of a name and ↵ switches to it; a new name and ↵ starts
    /// that branch.
    #[gpui::test]
    #[cfg(unix)]
    fn branches_switch_and_start_from_the_list(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-branch-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git").arg("-C").arg(&dir).args(args).output();
            out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        };
        if git(&["init", "-q", "-b", "main"]).is_err() {
            return; // No git here.
        }
        git(&["config", "user.name", "t"]).unwrap();
        git(&["config", "user.email", "t@t"]).unwrap();
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        git(&["add", "-A"]).unwrap();
        git(&["commit", "-qm", "first"]).unwrap();
        git(&["branch", "feature/login"]).unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let settle = |cx: &mut gpui::VisualTestContext| {
            cx.executor().advance_clock(Duration::from_millis(500));
            cx.run_until_parked();
        };
        settle(cx);
        workspace.update_in(cx, |w, window, cx| w.switch_branch(&SwitchBranch, window, cx));
        settle(cx);
        cx.simulate_input("login");
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert_eq!(git(&["branch", "--show-current"]).unwrap(), "feature/login");
        workspace.update_in(cx, |w, window, cx| w.switch_branch(&SwitchBranch, window, cx));
        settle(cx);
        cx.simulate_input("fix-typo");
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert_eq!(git(&["branch", "--show-current"]).unwrap(), "fix-typo");
        std::fs::remove_dir_all(&dir).ok();
    }

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

    #[gpui::test]
    fn back_and_forward_retrace_the_jumps(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-back-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lines: String = (1..=40).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join("a.txt"), &lines).unwrap();
        std::fs::write(dir.join("b.txt"), &lines).unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let at = |w: &Workspace, cx: &App| {
            let e = w.active_editor().unwrap().read(cx);
            (e.file_name(), e.caret_point().0)
        };
        let line =
            |n: u32| lsp_types::Range { start: lsp_types::Position::new(n, 0), end: lsp_types::Position::new(n, 0) };
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(dir.join("a.txt"), window, cx);
            w.go_to(dir.join("a.txt"), line(5), window, cx);
            // A definition in another file, then a line further down there.
            w.go_to(dir.join("b.txt"), line(20), window, cx);
            w.go_to(dir.join("b.txt"), line(30), window, cx);
            w.go_back(&GoBack, window, cx);
            assert_eq!(at(w, cx), ("b.txt".into(), 20));
            w.go_back(&GoBack, window, cx);
            assert_eq!(at(w, cx), ("a.txt".into(), 5));
            w.go_back(&GoBack, window, cx);
            assert_eq!(at(w, cx), ("a.txt".into(), 0));
            w.go_forward(&GoForward, window, cx);
            w.go_forward(&GoForward, window, cx);
            assert_eq!(at(w, cx), ("b.txt".into(), 20));
            // Going somewhere new ends the way forward.
            w.go_to(dir.join("a.txt"), line(10), window, cx);
            w.go_forward(&GoForward, window, cx);
            assert_eq!(at(w, cx), ("a.txt".into(), 10));
        });
        // The keys: ⌃- and ⌃⇧- on macOS, Alt+← and Alt+→ elsewhere.
        let (back, forward) =
            if cfg!(target_os = "macos") { ("ctrl--", "ctrl-shift--") } else { ("alt-left", "alt-right") };
        cx.simulate_keystrokes(back);
        workspace.update(cx, |w, cx| assert_eq!(at(w, cx), ("b.txt".into(), 20)));
        cx.simulate_keystrokes(forward);
        workspace.update(cx, |w, cx| assert_eq!(at(w, cx), ("a.txt".into(), 10)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn files_save_by_themselves_when_asked(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-autosave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings { auto_save: AutoSave::AfterPause, ..Settings::default() });
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let disk = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        let a = workspace.update_in(cx, |w, window, cx| {
            w.open_file(dir.join("a.txt"), window, cx);
            w.active_editor().unwrap().clone()
        });
        // After a pause: saved a moment after typing stops, not before.
        a.update(cx, |e, cx| e.type_text_for_test(0, "1", cx));
        cx.run_until_parked();
        assert_eq!(disk("a.txt"), "a\n");
        cx.executor().advance_clock(AutoSave::PAUSE * 2);
        cx.run_until_parked();
        assert_eq!(disk("a.txt"), "1a\n");
        // When leaving: saved on switching to another file.
        cx.update(|_, cx| crate::settings::update(cx, |s| s.auto_save = AutoSave::WhenLeaving));
        a.update(cx, |e, cx| e.type_text_for_test(0, "2", cx));
        cx.executor().advance_clock(AutoSave::PAUSE * 2);
        cx.run_until_parked();
        assert_eq!(disk("a.txt"), "1a\n");
        workspace.update_in(cx, |w, window, cx| w.open_file(dir.join("b.txt"), window, cx));
        cx.run_until_parked();
        assert_eq!(disk("a.txt"), "21a\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn focus_mode_comes_and_goes_with_its_key(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-focus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| w.open_file(dir.join("a.txt"), window, cx));
        let keys = if cfg!(target_os = "macos") { "alt-cmd-enter" } else { "ctrl-alt-enter" };
        cx.simulate_keystrokes(keys);
        workspace.update(cx, |w, cx| {
            assert!(w.focus_mode);
            // The sidebar setting stays as it was, for when Focus mode ends.
            assert!(cx.global::<Settings>().sidebar_visible);
        });
        cx.simulate_keystrokes(keys);
        workspace.update(cx, |w, _| assert!(!w.focus_mode));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn f8_walks_the_problems_then_the_next_file(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-f8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lines: String = (1..=10).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join("a.txt"), &lines).unwrap();
        std::fs::write(dir.join("b.txt"), &lines).unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let problem = |line: u32, severity| lsp_types::Diagnostic {
            range: lsp_types::Range {
                start: lsp_types::Position::new(line, 0),
                end: lsp_types::Position::new(line, 4),
            },
            severity: Some(severity),
            message: format!("problem on {line}"),
            ..Default::default()
        };
        use lsp_types::DiagnosticSeverity as S;
        let at = |w: &Workspace, cx: &App| {
            let e = w.active_editor().unwrap().read(cx);
            (e.file_name(), e.caret_point().0)
        };
        workspace.update_in(cx, |w, window, cx| {
            let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
            w.lsp.update(cx, |lsp, _| {
                // A hint isn't a stop: only errors and warnings.
                lsp.set_diagnostics(a.clone(), vec![problem(5, S::WARNING), problem(2, S::ERROR), problem(7, S::HINT)]);
                lsp.set_diagnostics(b.clone(), vec![problem(1, S::ERROR)]);
            });
            w.open_file(a, window, cx);
        });
        let mut stops = Vec::new();
        for _ in 0..4 {
            cx.simulate_keystrokes("f8");
            stops.push(workspace.update(cx, |w, cx| at(w, cx)));
        }
        assert_eq!(stops, [("a.txt".into(), 2), ("a.txt".into(), 5), ("b.txt".into(), 1), ("a.txt".into(), 2)]);
        cx.simulate_keystrokes("shift-f8");
        workspace.update(cx, |w, cx| assert_eq!(at(w, cx), ("b.txt".into(), 1)));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Files dropped on a Markdown file: links where they land; one from outside the
    /// project copied next to it first.
    #[gpui::test]
    fn files_dropped_on_markdown_become_links(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-md-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (project, outside) = (dir.join("project"), dir.join("outside"));
        std::fs::create_dir_all(project.join("docs")).unwrap();
        std::fs::create_dir_all(project.join("assets")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(project.join("docs/notes.md"), "# Notes\n\n").unwrap();
        std::fs::write(project.join("assets/logo.png"), "png").unwrap();
        std::fs::write(outside.join("shot.png"), "png").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = project.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(project.join("docs/notes.md"), window, cx);
            let editor = w.active_editor().unwrap().clone();
            let paths = [project.join("assets/logo.png"), outside.join("shot.png")];
            w.link_dropped(editor.clone(), &paths, 9, cx);
            assert_eq!(editor.read(cx).buffer.to_string(), "# Notes\n\n![logo](../assets/logo.png)\n![shot](shot.png)");
        });
        assert!(project.join("docs/shot.png").is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Line spacing from the settings reaches the open files.
    #[gpui::test]
    fn line_spacing_applies_to_open_files(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-line-spacing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| w.open_file(dir.join("a.txt"), window, cx));
        let height = |cx: &mut gpui::VisualTestContext| {
            workspace.read_with(cx, |w, cx| w.active_editor().unwrap().read(cx).line_height())
        };
        assert_eq!(height(cx), px(24.));
        cx.update(|_, cx| settings::update(cx, |s| s.line_spacing = crate::settings::LineSpacing::Relaxed));
        cx.run_until_parked();
        assert_eq!(height(cx), px(28.));
        // From ⌘K: "spacing", then → for the next one (round to Compact after Relaxed).
        cx.update(|_, cx| crate::keymap::register(crate::keymap::Keymap::Null, cx));
        workspace.update_in(cx, |w, window, cx| window.focus(&w.focus_handle(cx)));
        cx.simulate_keystrokes("cmd-k");
        cx.run_until_parked();
        cx.simulate_input("spacing");
        cx.executor().advance_clock(std::time::Duration::from_millis(300));
        cx.run_until_parked();
        cx.simulate_keystrokes("right");
        cx.run_until_parked();
        let spacing = |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| cx.global::<Settings>().line_spacing);
        assert_eq!(spacing(cx), crate::settings::LineSpacing::Compact);
        cx.simulate_keystrokes("right");
        cx.run_until_parked();
        assert_eq!(spacing(cx), crate::settings::LineSpacing::Normal);
        assert_eq!(height(cx), px(24.));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// ⌘⇧F with nothing selected starts from the latest search made in a file.
    #[gpui::test]
    fn project_search_starts_from_the_latest_search(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-search-shared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha beta\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| w.open_file(dir.join("a.txt"), window, cx));
        cx.update(|_, cx| crate::find_bar::remember_search("beta", cx));
        workspace.update_in(cx, |w, window, cx| w.search_project(&SearchProject, window, cx));
        cx.run_until_parked();
        workspace.read_with(cx, |w, cx| assert!(w.project_search.read(cx).has_query(cx)));
        assert_eq!(workspace.read_with(cx, |w, cx| w.project_search_text(cx)), None, "the field already has it");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Go to Last Edit opens the file last edited, at the edit; Back comes back.
    #[gpui::test]
    fn go_to_last_edit_goes_back_to_the_typing(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-last-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join("b.txt"), "three\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(dir.join("a.txt"), window, cx);
            let editor = w.active_editor().unwrap().clone();
            window.focus(&editor.focus_handle(cx));
            editor.update(cx, |e, cx| e.set_caret_point((1, 3), cx));
        });
        cx.simulate_input("!");
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(dir.join("b.txt"), window, cx);
            window.focus(&w.focus_handle(cx));
        });
        let place = |cx: &mut gpui::VisualTestContext| {
            workspace.read_with(cx, |w, cx| {
                let e = w.active_editor().unwrap().read(cx);
                (e.file_name(), e.caret_point())
            })
        };
        cx.dispatch_action(GoToLastEdit);
        assert_eq!(place(cx), ("a.txt".into(), (1, 4)));
        cx.dispatch_action(GoBack);
        assert_eq!(place(cx).0, "b.txt");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Compare with File…: the files' list, a file picked, the open one against it.
    #[gpui::test]
    fn a_file_is_compared_with_another(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-compare-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("new.txt"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(dir.join("old.txt"), "one\ntwo\nthree\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(dir.join("new.txt"), window, cx);
            w.pick_file_to_compare(window, cx);
        });
        cx.executor().advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        cx.simulate_input("old");
        cx.executor().advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        workspace.update(cx, |w, cx| {
            let editor = w.active_editor().unwrap().read(cx);
            assert_eq!(editor.file_name(), "new.txt", "still the same file");
            assert!(editor.in_review(), "its difference with old.txt shows");
            assert!(w.compare_from.is_none());
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn the_status_bar_counts_what_is_selected(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(crate::buffer::Buffer::from_text("one two\nthree\nfour\n"), Some("x.rs".into()), cx)
        });
        let label = |cx: &mut gpui::VisualTestContext, anchor: usize, head: usize| {
            e.update(cx, |e, _| {
                e.selection = crate::editor::Selection { anchor, head };
                let (line, col) = e.caret_point();
                position_label(e, line, col)
            })
        };
        assert_eq!(label(cx, 4, 4), "Ln 1, Col 5");
        assert_eq!(label(cx, 4, 7), "Ln 1, Col 8 · 3 selected");
        assert_eq!(label(cx, 0, 14), "Ln 3, Col 1 · 2 lines selected");
        assert_eq!(label(cx, 16, 2), "Ln 1, Col 3 · 3 lines selected");
        // In prose, words: in the file, or in the selection.
        let (notes, cx) = cx.add_window_view(|_, cx| {
            Editor::new(crate::buffer::Buffer::from_text("# Plan\n\n- buy milk\n"), Some("notes.md".into()), cx)
        });
        let label = |cx: &mut gpui::VisualTestContext, anchor: usize, head: usize| {
            notes.update(cx, |e, _| {
                e.selection = crate::editor::Selection { anchor, head };
                position_label(e, 0, 0)
            })
        };
        assert_eq!(label(cx, 0, 0), "Ln 1, Col 1 · 3 words");
        assert_eq!(label(cx, 10, 18), "Ln 1, Col 1 · 2 words selected");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(999), "999");
    }

    #[test]
    fn changes_are_read_as_a_diff() {
        let changes = vec![
            ("src/a.rs".to_string(), Some("one\ntwo\n".to_string()), Some("one\n2\n".to_string())),
            ("new.txt".to_string(), None, Some("hello\n".to_string())),
        ];
        let diff = changes_as_diff(&changes, 10_000);
        assert!(diff.contains("--- a/src/a.rs\n+++ b/src/a.rs\n"), "{diff}");
        assert!(diff.contains("-two\n+2\n") && diff.contains("+hello\n"), "{diff}");
        assert!(changes_as_diff(&changes, 20).ends_with("[… more changes left out]\n"));
    }

    #[test]
    fn code_is_copied_as_a_markdown_block() {
        let lines = ["    if a {".to_string(), "        b();".into(), "    }".into()];
        assert_eq!(
            code_block("src/a.rs", "rs", 3, 5, &lines),
            "`src/a.rs` lines 3–5\n\n```rs\nif a {\n    b();\n}\n```\n"
        );
        // A fence in the code: a longer one around it.
        let md = ["```".to_string(), "x".into(), "```".into()];
        assert!(code_block("notes.md", "md", 1, 3, &md).contains("\n````md\n```\nx\n```\n````\n"));
        assert!(code_block("a.py", "py", 7, 7, &["pass".into()]).starts_with("`a.py` line 7"));
    }

    #[test]
    fn terminal_tabs_are_named_after_their_folder() {
        assert_eq!(Workspace::terminal_label("ada@mac:~/code/null", 0), "null");
        assert_eq!(Workspace::terminal_label("ada@mac:/tmp/", 1), "tmp");
        assert_eq!(Workspace::terminal_label("", 2), "Terminal 3");
        assert_eq!(Workspace::terminal_name("ada@mac:~/code/null"), "null");
        assert_eq!(Workspace::terminal_name("cargo run"), "cargo run");
    }

    #[gpui::test]
    fn a_tab_s_menu_closes_to_the_right_and_copies_its_path(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-tab-menu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        for name in ["a.txt", "b.txt", "src/c.txt"] {
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
        workspace.update_in(cx, |w, window, cx| {
            for name in ["a.txt", "b.txt", "src/c.txt"] {
                w.open_file(dir.join(name), window, cx);
            }
            use TabMenuItem::*;
            let (first, last) = (w.tabs[0].editor.clone(), w.tabs[2].editor.clone());
            // The last tab has nothing to its right.
            assert!(!w.tab_menu_items(&last, cx).contains(&CloseToTheRight));
            assert_eq!(
                w.tab_menu_items(&first, cx),
                [Close, CloseOthers, CloseToTheRight, CopyPath, CopyRelativePath, Reveal, OtherSide]
            );
            w.tab_menu = Some(TabMenu { editor: last.clone(), position: Default::default() });
            w.run_tab_menu_item(CopyRelativePath, window, cx);
            assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some("src/c.txt".into()));
            w.tab_menu = Some(TabMenu { editor: first.clone(), position: Default::default() });
            w.run_tab_menu_item(CloseToTheRight, window, cx);
            assert_eq!(w.tabs.len(), 1);
            assert!(w.tab_menu.is_none());
            window.focus(&w.focus_handle(cx));
        });
        // The same from ⌘K, for the open file.
        let clipboard = |cx: &mut gpui::VisualTestContext| cx.read_from_clipboard().and_then(|c| c.text());
        cx.dispatch_action(CopyRelativeFilePath);
        assert_eq!(clipboard(cx), Some("a.txt".into()));
        cx.dispatch_action(CopyFilePath);
        assert_eq!(clipboard(cx), Some(dir.join("a.txt").display().to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn renaming_a_file_without_a_server_just_renames_it(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.txt"), "hello\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let (from, to) = (dir.join("notes.txt"), dir.join("ideas.txt"));
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(from.clone(), window, cx);
            w.rename_file(from.clone(), to.clone(), cx);
        });
        cx.run_until_parked();
        assert!(!from.exists() && to.exists());
        workspace.update(cx, |w, cx| {
            assert_eq!(w.active_editor().unwrap().read(cx).path(), Some(to.as_path()));
        });
        // From ⌘K: the name is asked for in the files, as there.
        workspace.update_in(cx, |w, window, cx| window.focus(&w.focus_handle(cx)));
        cx.dispatch_action(RenameFile);
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("plans.txt");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let renamed = dir.join("plans.txt");
        assert!(!to.exists() && renamed.exists());
        workspace.update(cx, |w, cx| {
            assert_eq!(w.active_editor().unwrap().read(cx).path(), Some(renamed.as_path()));
        });
        // Move File to Trash… asks first (answered Cancel: the test leaves the real Trash alone).
        workspace.update_in(cx, |w, window, cx| window.focus(&w.focus_handle(cx)));
        cx.dispatch_action(TrashFile);
        cx.run_until_parked();
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert!(renamed.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn unsaved_work_comes_back_after_a_crash(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        // Typing in a file and in a new one, then the backup is written…
        let root = dir.clone();
        let (first, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        first.update_in(cx, |w, window, cx| {
            w.open_file(dir.join("a.txt"), window, cx);
            w.active_editor().unwrap().update(cx, |e, cx| e.type_text_for_test(0, "zero ", cx));
            w.new_untitled(&NewUntitled, window, cx);
            w.active_editor().unwrap().update(cx, |e, cx| e.type_text_for_test(0, "a draft", cx));
            w.write_backups(cx);
        });
        cx.run_until_parked();
        assert_eq!(crate::session::load_backups(&dir).len(), 2);
        // …and Null goes away without asking (a crash). Opening the project again:
        let root = dir.clone();
        let second = cx.update(|window, cx| cx.new(|cx| Workspace::new(root, window, cx)));
        second.update_in(cx, |w, window, cx| {
            w.restore_session(crate::session::Session::default(), window, cx);
            let texts: Vec<(Option<String>, String, bool)> = w
                .tabs
                .iter()
                .map(|t| {
                    let e = t.editor.read(cx);
                    (
                        e.path().map(|_| e.file_name()).filter(|_| e.path().is_some()),
                        e.buffer.to_string(),
                        e.buffer.is_dirty(),
                    )
                })
                .collect();
            assert!(texts.contains(&(Some("a.txt".into()), "zero one\n".into(), true)), "{texts:?}");
            assert!(texts.contains(&(None, "a draft".into(), true)), "{texts:?}");
            // The file itself wasn't touched.
            assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\n");
            // Leaving the project properly (after the save question) lets the backup go.
            let elsewhere = dir.join("elsewhere");
            std::fs::create_dir_all(&elsewhere).unwrap();
            w.finish_close(CloseAction::SwitchProject(elsewhere), window, cx);
        });
        cx.run_until_parked();
        assert!(crate::session::load_backups(&dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Renaming a folder of images, then moving a guide up a level: the Markdown links to
    /// them, and the guide's own, still point where they should; the open README too.
    #[gpui::test]
    fn markdown_links_follow_renames(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-markdown-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("img")).unwrap();
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("img/a.png"), "x").unwrap();
        std::fs::write(dir.join("README.md"), "![a](img/a.png) and [guide](docs/guide.md#run)\n").unwrap();
        std::fs::write(dir.join("docs/guide.md"), "Back to [readme](../README.md), ![a](../img/a.png)\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(dir.join("README.md"), window, cx);
            w.rename_file(dir.join("img"), dir.join("pics"), cx);
        });
        cx.run_until_parked();
        workspace.update_in(cx, |w, _, cx| w.rename_file(dir.join("docs/guide.md"), dir.join("guide.md"), cx));
        cx.run_until_parked();
        // The open README changed in its tab (saved with the rest).
        workspace.update(cx, |w, cx| {
            let readme = w.active_editor().unwrap().read(cx).buffer.to_string();
            assert_eq!(readme, "![a](pics/a.png) and [guide](guide.md#run)\n");
        });
        assert_eq!(
            std::fs::read_to_string(dir.join("guide.md")).unwrap(),
            "Back to [readme](README.md), ![a](pics/a.png)\n"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn a_file_moved_to_a_folder_keeps_its_tab(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-move-tab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("notes.md"), "hello\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let (from, to) = (dir.join("notes.md"), dir.join("docs/notes.md"));
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(from.clone(), window, cx);
            // What dropping it on the folder asks for.
            w.rename_file(from.clone(), to.clone(), cx);
        });
        cx.run_until_parked();
        assert!(!from.exists() && to.is_file());
        workspace.update(cx, |w, cx| assert_eq!(w.active_editor().unwrap().read(cx).path(), Some(to.as_path())));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn quitting_asks_every_window_with_unsaved_changes(cx: &mut gpui::TestAppContext) {
        let make = |name: &str| {
            let dir = std::env::temp_dir().join(format!("null-quit-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("a.txt"), "a\n").unwrap();
            dir
        };
        let (first, second) = (make("one"), make("two"));
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let open = |cx: &mut gpui::TestAppContext, dir: &Path| {
            let root = dir.to_path_buf();
            let window = cx.add_window(|window, cx| Workspace::new(root, window, cx));
            let file = dir.join("a.txt");
            window
                .update(cx, |w, window, cx| {
                    w.open_file(file, window, cx);
                    w.active_editor().unwrap().update(cx, |e, cx| e.type_text_for_test(0, "edit ", cx));
                })
                .unwrap();
            window
        };
        let (one, two) = (open(cx, &first), open(cx, &second));
        // ⌘Q in the first window: it asks about its file…
        one.update(cx, |w, window, cx| w.quit(&Quit, window, cx)).unwrap();
        cx.run_until_parked();
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Don't Save");
        cx.run_until_parked();
        // …then the second window asks about its own, rather than Null quitting over it.
        assert!(one.update(cx, |w, _, _| w.quitting).unwrap());
        assert!(cx.has_pending_prompt());
        assert!(!two.update(cx, |w, _, _| w.quitting).unwrap());
        cx.simulate_prompt_answer("Don't Save");
        cx.run_until_parked();
        std::fs::remove_dir_all(&first).ok();
        std::fs::remove_dir_all(&second).ok();
    }

    #[gpui::test]
    fn a_file_open_on_both_sides_stays_the_same_on_both(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-twins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let (left, right) = workspace.update_in(cx, |w, window, cx| {
            w.open_file(dir.join("a.txt"), window, cx);
            w.open_on_other_side(window, cx);
            assert!(w.is_split());
            (w.tabs[w.side_tabs(0)[0]].editor.clone(), w.tabs[w.side_tabs(1)[0]].editor.clone())
        });
        let text = |cx: &mut gpui::VisualTestContext, e: &Entity<Editor>| e.read_with(cx, |e, _| e.buffer.to_string());
        // Typing on the left shows on the right, and the other way round.
        left.update(cx, |e, cx| e.type_text_for_test(0, "zero\n", cx));
        cx.run_until_parked();
        assert_eq!(text(cx, &right), "zero\none\ntwo\n");
        right.update(cx, |e, cx| e.type_text_for_test(e.buffer.len_chars(), "three\n", cx));
        cx.run_until_parked();
        assert_eq!(text(cx, &left), "zero\none\ntwo\nthree\n");
        assert!(right.read_with(cx, |e, _| e.lsp_follower) && !left.read_with(cx, |e, _| e.lsp_follower));
        // Saving one saves both.
        left.update(cx, |e, cx| e.save_to_disk(cx));
        cx.run_until_parked();
        assert!(!right.read_with(cx, |e, _| e.buffer.is_dirty()));
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "zero\none\ntwo\nthree\n");
        // Closing the first copy: the other one takes over the language server.
        workspace.update_in(cx, |w, window, cx| {
            let ix = w.side_tabs(0)[0];
            w.remove_tab(ix, window, cx);
        });
        assert!(!right.read_with(cx, |e, _| e.lsp_follower));
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
