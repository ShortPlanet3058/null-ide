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
    MouseButton, MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels, Point, PromptLevel, ScrollStrategy,
    SharedString, Subscription, Task, UniformListScrollHandle, Window, WindowControlArea, actions, div, prelude::*, px,
    relative, svg, uniform_list,
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
        SwitchTab,
        SwitchTabBack,
        MoveTabRight,
        MoveTabLeft,
        MoveTabDown,
        MoveTabUp,
        OpenOnOtherSide,
        NewAiTask,
        ReviewChanges,
        CommitAll,
        UndoLastCommit,
        SetChangesAside,
        BringBackChanges,
        OpenInBrowser,
        FindTodos,
        RunSelectionInTerminal,
        PasteFromHistory,
        ExportHtml,
        ShowBookmarks,
        EditSnippets,
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
        CopyAsRichText,
        OrganizeImports,
        RevertToSaved,
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
        ToggleCodeLens,
        ShowServerLog,
        ToggleBracketColours,
        ToggleProblemsAtLineEnds,
        ToggleFocusMode,
        RunTask,
        RunTestAtCursor,
        RunTestsInFile,
        OpenPreviewToTheSide,
        OpenRecent,
        NewWindow,
        StartDebugging,
        ToggleStopOnErrors,
        StopDebugging,
        StepOver,
        StepInto,
        StepOut,
        AddWatch,
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
        OpenProjectSettings,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        CompactLineSpacing,
        NormalLineSpacing,
        RelaxedLineSpacing,
        UseNullTheme,
        NewOwnTheme,
        UseAshTheme,
        UseMidnightTheme,
        UseMossTheme,
        UsePaperTheme,
        UseDuneTheme,
        SearchProject,
        ReplaceInProject,
        ShowFiles,
        ShowOutline,
        ShowTests,
        ToggleAutocomplete,
        ToggleTerminal,
        NewTerminal,
        NextTerminal,
        SplitTerminal,
        GoToLine,
        GoToSymbol,
        GoToSymbolInProject,
        NewUntitled,
        SaveAs,
        SaveAll,
        ReopenClosedTab,
        TogglePinTab,
        RenameTerminal,
        ConfirmTerminalName,
        CancelTerminalName,
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
        KeyBinding::new("enter", AddWatch, Some("DebugWatch")),
        KeyBinding::new("enter", ConfirmTerminalName, Some("TerminalName")),
        KeyBinding::new("escape", CancelTerminalName, Some("TerminalName")),
        KeyBinding::new("f8", NextProblem, ctx),
        KeyBinding::new("shift-f8", PreviousProblem, ctx),
        KeyBinding::new("ctrl-tab", SwitchTab, ctx),
        KeyBinding::new("ctrl-shift-tab", SwitchTabBack, ctx),
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
            // As in iTerm: in the terminal, another beside it.
            KeyBinding::new("cmd-d", SplitTerminal, Some("Terminal")),
        ]);
    } else {
        keys.extend([KeyBinding::new("alt-left", GoBack, ctx), KeyBinding::new("alt-right", GoForward, ctx)]);
    }
    cx.bind_keys(keys);
}

/// What a tab's right-click menu offers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TabMenuItem {
    Pin,
    Unpin,
    Close,
    CloseOthers,
    CloseToTheRight,
    CopyPath,
    CopyRelativePath,
    Reveal,
    OpenInTerminal,
    OtherSide,
    MoveRight,
    MoveDown,
    /// Back to the first side: left, or above.
    MoveBack,
    History,
    CopyLink,
}

impl TabMenuItem {
    fn label(self) -> &'static str {
        match self {
            TabMenuItem::Pin => "Pin Tab",
            TabMenuItem::Unpin => "Unpin Tab",
            TabMenuItem::Close => "Close",
            TabMenuItem::CloseOthers => "Close Others",
            TabMenuItem::CloseToTheRight => "Close Tabs to the Right",
            TabMenuItem::CopyPath => "Copy Path",
            TabMenuItem::CopyRelativePath => "Copy Relative Path",
            TabMenuItem::Reveal => crate::file_tree::REVEAL_LABEL,
            TabMenuItem::OpenInTerminal => "Open in Terminal",
            TabMenuItem::OtherSide => "Open on the Other Side Too",
            TabMenuItem::MoveRight => "Move to the Right",
            TabMenuItem::MoveDown => "Move Below",
            TabMenuItem::MoveBack => "Move Back",
            TabMenuItem::History => "Show History",
            TabMenuItem::CopyLink => "Copy Link to Line",
        }
    }

    /// Items that start a group get a line above them.
    fn starts_group(self) -> bool {
        matches!(
            self,
            TabMenuItem::Close
                | TabMenuItem::CopyPath
                | TabMenuItem::OtherSide
                | TabMenuItem::MoveRight
                | TabMenuItem::MoveBack
        )
    }
}

/// A tab's right-click menu: the tab, and where the click was.
struct TabMenu {
    editor: Entity<Editor>,
    position: Point<Pixels>,
}

/// The program `cargo build` makes for a project: its first `[[bin]]`, or the package
/// named in Cargo.toml, in target/debug.
/// The program `cargo build --message-format=json` says it built: the one named as
/// `expected` is (its file name), else the only one; None when it built none, or several
/// and none of them is the one expected (which, isn't for Null to guess).
fn built_program(messages: &str, expected: Option<&Path>) -> Option<PathBuf> {
    let programs: Vec<PathBuf> = messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|m| m["reason"] == "compiler-artifact")
        .filter(|m| m["target"]["kind"].as_array().is_some_and(|kinds| kinds.iter().any(|k| k == "bin")))
        .filter_map(|m| m["executable"].as_str().map(PathBuf::from))
        .collect();
    let wanted = expected.and_then(Path::file_name);
    let only = (programs.len() == 1).then(|| &programs[0]);
    programs.iter().find(|p| p.file_name() == wanted).or(only).cloned()
}

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
    /// Files you saved while it ran: their changes aren't the task's alone.
    yours: HashSet<PathBuf>,
    _task: Option<Task<()>>,
}

/// How a test did when last run from the list (or a run that printed its name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestStatus {
    Running,
    Passed,
    Failed,
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

/// A terminal being named: which one, the field, and the watch on the field losing focus.
struct TerminalRename {
    terminal: gpui::EntityId,
    input: Entity<crate::text_input::TextInput>,
    _blur: Subscription,
}

struct Tab {
    editor: Entity<Editor>,
    /// Which side it's on: 0 left (or the only one), 1 right.
    side: usize,
    /// Kept at the front of its side, and out of "Close Others" and the like.
    pinned: bool,
    /// Opened in passing (one click in the files): the next file so opened takes its place,
    /// until it's edited, double-clicked or pinned.
    passing: bool,
    _subscriptions: [Subscription; 2],
}

pub struct Workspace {
    focus_handle: FocusHandle,
    tree: Entity<FileTree>,
    lsp: Entity<LspStore>,
    project_search: Entity<ProjectSearch>,
    /// On while the sidebar shows project search instead of files.
    sidebar_search: Transition,
    /// The sidebar shows the open file's outline instead of the files (when not searching).
    sidebar_outline: bool,
    /// The sidebar shows the project's tests (see `crate::test_at::discover`).
    sidebar_tests: bool,
    /// The project's own Python environment (`.venv`), if it has one.
    python_env: Option<PathBuf>,
    /// The open file's (its package's own, in a monorepo), and which file that was for.
    python_env_here: Option<(PathBuf, Option<PathBuf>)>,
    /// The tests found (None: not looked for yet), and the search for them.
    tests: Option<std::rc::Rc<Vec<crate::test_at::FileTests>>>,
    tests_task: Option<Task<()>>,
    /// How each test (file, name) did when last run.
    test_status: std::collections::HashMap<(PathBuf, String), TestStatus>,
    /// The tests run from the list, and the command running them.
    tests_running: Option<(String, Vec<(PathBuf, String)>)>,
    tests_scroll: UniformListScrollHandle,
    /// The outline last read: whose (an editor), at which revision of its text.
    outline: Option<(gpui::EntityId, u64, std::rc::Rc<Vec<crate::outline::Item>>)>,
    /// When the outline was last worked out, and the redraw asked for once typing pauses
    /// (a big file's isn't worked out again on every keystroke).
    outline_at: Instant,
    outline_later: Option<Task<()>>,
    outline_scroll: UniformListScrollHandle,
    /// The item the caret was in when last drawn: the list follows it to another.
    outline_followed: Option<(gpui::EntityId, usize)>,
    tabs: Vec<Tab>,
    /// The tab with the keyboard: the one shown on the side being worked in.
    active: Option<usize>,
    /// The tab each side shows.
    shown: [Option<Entity<Editor>>; 2],
    /// The tabs' editors, the one used last first (for ⌃Tab).
    used: Vec<gpui::EntityId>,
    /// While ⌃ is held after ⌃Tab: how far down `used` it has gone. Letting go of ⌃ makes
    /// the tab reached the last used.
    switching: Option<usize>,
    /// How much of the width the left side takes when split (of the height, stacked).
    split_ratio: f32,
    /// The two sides one above the other, rather than side by side.
    stacked: bool,
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
    /// When git's status was last asked (see `refresh_git_status`).
    git_status_at: Instant,
    /// The list of changes is open: opening a file from it shows its changes.
    git_listing: bool,
    /// The history a list shows (⌥ picking one compares the file with it), and its file.
    history: Option<(PathBuf, Vec<Past>)>,
    /// While Paste from History is open: what its rows stand for.
    clipboard_list: Option<Vec<gpui::ClipboardItem>>,
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
    /// Commands run from ⌘⇧B, the last first, and the task each was (by name): a task
    /// naming the file comes back for the file at hand, not the one it ran on.
    recent_runs: Vec<(String, Option<String>)>,
    /// The debugger, and its panel (shown while debugging, and after, until closed).
    debugger: Entity<crate::debugger::Debugger>,
    debug_panel_open: bool,
    debug_output_scroll: gpui::ScrollHandle,
    /// While paused, the panel shows the variables and the calls; this shows the output instead.
    debug_show_output: bool,
    /// The paused call's variables the code shows, with values worth showing.
    debug_locals: Vec<(String, String)>,
    /// Renames asked for and waiting their turn (see `queue_rename`), and whether one is
    /// under way.
    renames: std::collections::VecDeque<(PathBuf, PathBuf)>,
    renaming: bool,
    /// The problems the last command in the terminal reported (a build's errors), and
    /// that command (`cargo build`).
    reported: Vec<(PathBuf, lsp_types::Diagnostic)>,
    reported_by: String,
    /// The shown terminal's new name, being typed.
    terminal_rename: Option<TerminalRename>,
    /// Where an expression to watch is typed, under the variables while paused.
    debug_watch: Entity<crate::text_input::TextInput>,
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
    /// Unsaved work was just kept for next time: the backup stays.
    keeping_unsaved: bool,
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
    /// A notice for when Null comes back to the front (a command finished meanwhile).
    notice_on_return: Option<String>,
    /// The terminal, once opened. It keeps running while the panel is hidden.
    /// The shells open in the terminal panel, and the one shown.
    terminals: Vec<(Entity<TerminalView>, Subscription)>,
    active_terminal: usize,
    /// Two terminals side by side (left, right), shown while one of them is the current.
    terminal_pair: Option<(gpui::EntityId, gpui::EntityId)>,
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
        // This project's own settings, over yours, while its window is in front.
        crate::settings::use_project(&root, cx);
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
                    this.open_file_passing(path.clone(), window, cx);
                    window.focus(&this.tree.focus_handle(cx));
                }
                FileTreeEvent::Renamed { from, to } => this.paths_renamed(from, to, cx),
                FileTreeEvent::RenameRequested { from, to } => this.queue_rename(from.clone(), to.clone(), cx),
                FileTreeEvent::Trashed(path) => this.path_trashed(path, window, cx),
                FileTreeEvent::OpenTerminal(dir) => this.open_terminal_in(dir.clone(), window, cx),
                FileTreeEvent::FindInFolder(dir) => this.find_in_folder(dir, window, cx),
                // Two files picked in the tree: the first opened, compared with the second.
                FileTreeEvent::Compare(first, second) => {
                    this.open_file(first.clone(), window, cx);
                    if let Some(editor) = this.active_editor().cloned() {
                        this.compare_with_file(&editor, second, cx);
                    }
                }
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
                // Files with errors stand out in the files.
                crate::lsp_store::LspEvent::DiagnosticsChanged => {
                    let errors = this.lsp.read(cx).files_with_errors();
                    this.tree.update(cx, |tree, cx| tree.set_errors(errors, cx));
                }
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
            } else {
                // ⌃ let go elsewhere (another Space, Mission Control): the tab reached stays.
                this.settle_switch();
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
            sidebar_outline: false,
            sidebar_tests: false,
            python_env: None,
            python_env_here: None,
            tests: None,
            tests_task: None,
            test_status: std::collections::HashMap::new(),
            tests_running: None,
            tests_scroll: UniformListScrollHandle::new(),
            outline: None,
            outline_at: Instant::now(),
            outline_later: None,
            outline_scroll: UniformListScrollHandle::new(),
            outline_followed: None,
            tabs: Vec::new(),
            shown: [None, None],
            split_ratio: 0.5,
            used: Vec::new(),
            switching: None,
            stacked: false,
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
            terminal_pair: None,
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
            keeping_unsaved: false,
            view_before_preview: None,
            ai_task: None,
            git_status: Vec::new(),
            git_status_task: None,
            git_status_at: Instant::now() - Duration::from_secs(10),
            git_listing: false,
            history: None,
            clipboard_list: None,
            pending_branches: Vec::new(),
            recent_runs: Vec::new(),
            pending_tasks: Vec::new(),
            pending_projects: Vec::new(),
            debugger: debugger.clone(),
            debug_panel_open: false,
            debug_output_scroll: gpui::ScrollHandle::new(),
            debug_show_output: false,
            debug_locals: Vec::new(),
            debug_watch: cx.new(|cx| crate::text_input::TextInput::new("Watch an expression", cx)),
            terminal_rename: None,
            reported: Vec::new(),
            renames: Default::default(),
            renaming: false,
            reported_by: String::new(),
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
            notice_on_return: None,
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
        // The Mac going light or dark: the theme follows, when Settings say to.
        workspace
            ._subscriptions
            .push(cx.observe_window_appearance(window, |_, _, cx| crate::settings::appearance_changed(cx)));
        workspace._subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                let root = this.tree.read(cx).root().to_path_buf();
                crate::settings::use_project(&root, cx);
            }
            if window.is_window_active()
                && let Some(message) = this.notice_on_return.take()
            {
                this.show_notice(message, cx);
            }
            if !window.is_window_active() {
                // Off to another app: files save now when they save by themselves.
                // As files save by themselves: the line being typed on keeps its spaces, and
                // one changed on disk waits for a save that asks.
                if cx.global::<Settings>().auto_save != AutoSave::Off {
                    let editors: Vec<Entity<Editor>> = this.tabs.iter().map(|t| t.editor.clone()).collect();
                    for editor in &editors {
                        Self::save_if_named(editor, cx);
                    }
                }
                this.save_session(cx);
            }
        }));
        workspace.python_env = crate::python_env::find(workspace.tree.read(cx).root());
        // settings.json couldn't be read at launch: said, not left to pass for the defaults.
        if let Some(why) = Settings::unreadable() {
            workspace.show_notice(
                format!("settings.json can't be read ({why}): the defaults are used until it's put right"),
                cx,
            );
        }
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
                    bookmarks: editor.bookmarks.clone(),
                    language: editor.chosen_language().map(str::to_string),
                    pinned: tab.pinned,
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
            stacked: self.is_split() && self.stacked,
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
                    editor.bookmarks = tab.bookmarks.iter().copied().filter(|&l| l < lines).collect();
                    if let Some(name) = &tab.language {
                        editor.choose_language(crate::editor::language_by_name(name), cx);
                    }
                    editor.restore_view(tab.line, tab.column, tab.top_line, cx)
                });
            }
            // The tab opened for it (a file gone since opens none: nothing to pin).
            if tab.pinned
                && let Some(opened) = self
                    .tabs
                    .iter_mut()
                    .find(|t| t.side == side && t.editor.read(cx).path() == Some(tab.path.as_path()))
            {
                opened.pinned = true;
            }
        }
        self.keep_pinned_first();
        if let Some(ratio) = session.split_ratio {
            self.split_ratio = ratio.clamp(0.2, 0.8);
        }
        self.stacked = session.stacked;
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
                // Its beginning, read from the rope (not a copy of the whole file first).
                let text: String = tab.editor.read(cx).buffer.rope().chars().take(OPEN_FILE_CHARS).collect();
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
        // The project's settings, changed (by hand, by another app): in use as they are now.
        let root = self.tree.read(cx).root().to_path_buf();
        // (Only when they're the ones in use: another window's in front, they wait for this one.)
        // An environment made or removed (`python -m venv .venv`): known from now.
        // (Or Poetry's, kept outside: known again when the project's Poetry files change.)
        if visible.iter().any(|p| crate::python_env::may_change(&root, p)) {
            self.python_env_changed(cx);
        }
        let settings_changed = visible.contains(&crate::settings::project_file(&root))
            || visible.contains(&crate::settings::vscode_file(&root));
        if settings_changed && crate::settings::project_in_use(&root) {
            crate::settings::use_project(&root, cx);
        }
        let changed: HashSet<&PathBuf> = visible.iter().collect();
        for tab in &self.tabs {
            let Some(path) = tab.editor.read(cx).path().map(Path::to_path_buf) else { continue };
            if changed.contains(&path) {
                tab.editor.update(cx, |editor, cx| editor.reload_from_disk(cx));
            } else if visible.iter().any(|v| path.starts_with(v)) {
                // Its folder changed (deleted, renamed): only whether the file is still there.
                tab.editor.update(cx, |editor, cx| editor.check_missing(cx));
            }
        }
        // What git counts: ignored files (a build's output, node_modules) change nothing it
        // says, unless git has them all the same (added by force: one it lists, or one open).
        let root = self.tree.read(cx).root().to_path_buf();
        let open: HashSet<PathBuf> =
            self.tabs.iter().filter_map(|t| t.editor.read(cx).path().map(Path::to_path_buf)).collect();
        let counted: Vec<PathBuf> = visible
            .iter()
            .filter(|p| {
                !crate::project_index::is_ignored(&self.ignore_rules, &root, p)
                    || open.contains(*p)
                    || self.git_status.iter().any(|(listed, _)| listed == *p)
            })
            .cloned()
            .collect();
        // Moves among ignored files too: a tab open on one follows it.
        self.follow_moves(&visible, cx);
        cx.notify();
        if git_changed {
            self.refresh_git(cx);
        } else if !counted.is_empty() {
            self.refresh_git_status_for_changes(cx);
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
        self.refresh_git_status_after(Duration::from_millis(120), cx);
    }

    /// Files changed on disk: git's status again, but no more than about once a second
    /// while they keep changing (a build): each is a few git processes. (After Null's own
    /// git actions it's at once: what's shown next depends on it.)
    fn refresh_git_status_for_changes(&mut self, cx: &mut Context<Self>) {
        let since = self.git_status_at.elapsed();
        let wait = Duration::from_millis(120).max(Duration::from_secs(1).saturating_sub(since));
        self.refresh_git_status_after(wait, cx);
    }

    fn refresh_git_status_after(&mut self, wait: Duration, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        self.git_status_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, _| this.git_status_at = Instant::now()).ok();
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
                            // (A file that isn't text: no lines to count.)
                            // (Only what reads as no text: a folder, an unreadable file count as empty.)
                            let Ok((after, encoding)) = crate::encoding::read(&path).or_else(|e| {
                                if e.kind() == std::io::ErrorKind::InvalidData { Err(e) } else { Ok(Default::default()) }
                            }) else {
                                let not = crate::palette::NOT_TEXT;
                                return (path, status, not, not);
                            };
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
            editor.update(cx, |editor, cx| editor.save_by_itself(cx));
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
        const WRITING: &str = "Writing the commit message…";
        self.show_notice(WRITING.into(), cx);
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
                    // Written (or no longer wanted): not still said to be on its way.
                    this.update(cx, |this, cx| {
                        if this.notice.as_ref().is_some_and(|(n, _)| n == WRITING) {
                            this.notice = None;
                            cx.notify();
                        }
                    })
                    .ok();
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
        crate::system_clipboard::write(cx, gpui::ClipboardItem::new_string(block));
        let what =
            if first == last { format!("line {}", first + 1) } else { format!("lines {}–{}", first + 1, last + 1) };
        self.show_notice(format!("Copied {what} as a code block."), cx);
    }

    /// Markdown (the selection, or the whole file) on the clipboard as formatted text too:
    /// pasted in Mail, Notes or Docs it keeps its headings, bold and links; in code, it's
    /// the Markdown.
    fn copy_as_rich_text(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        // Code: in its colours, as it shows here.
        if !editor.read(cx).is_markdown() {
            let range = editor.read(cx).selection.range();
            let whole = range.is_empty();
            let range = if whole { 0..editor.read(cx).buffer.len_chars() } else { range };
            let source = editor.read(cx).buffer.slice(range.clone());
            let html = editor.update(cx, |e, cx| e.colored_html(range, cx));
            let item = gpui::ClipboardItem::new_string(source.clone());
            if !crate::markdown_html::copy_rich(&source, &html) {
                crate::system_clipboard::write(cx, item.clone());
            }
            crate::clipboard_history::remember(&item, cx);
            let what = if whole { "The file" } else { "The selection" };
            return self.show_notice(
                format!("{what} is copied in its colours: it pastes as it shows here in Keynote, Pages or Mail."),
                cx,
            );
        }
        let e = editor.read(cx);
        let range = e.selection.range();
        let whole = range.is_empty();
        let source = if whole { e.buffer.to_string() } else { e.buffer.slice(range) };
        let item = gpui::ClipboardItem::new_string(source.clone());
        if !crate::markdown_html::copy_rich(&source, &crate::markdown_html::fragment(&source)) {
            crate::system_clipboard::write(cx, item.clone());
        }
        crate::clipboard_history::remember(&item, cx);
        let what = if whole { "The document" } else { "The selection" };
        self.show_notice(format!("{what} is copied as rich text: it pastes formatted in Mail, Notes or Docs."), cx);
    }

    /// Revert to Saved: the file as it was last saved, as one step ⌘Z takes back.
    fn revert_to_saved(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        let (has_file, dirty) = {
            let e = editor.read(cx);
            (e.path().is_some_and(|p| p.exists()), e.buffer.is_dirty())
        };
        if !has_file {
            return self.show_notice("This file isn't saved anywhere yet.".into(), cx);
        }
        if !dirty {
            return self.show_notice("Nothing to revert: it's as it was saved.".into(), cx);
        }
        let back = editor.update(cx, |e, cx| {
            let back = e.revert_to_disk(cx);
            // Changed and changed back: the same text, saved.
            if back && e.buffer.is_dirty() {
                e.buffer.mark_saved();
                cx.emit(crate::editor::EditorEvent::Edited);
                cx.notify();
            }
            back
        });
        if !back {
            return self.show_notice("Couldn't read the saved file: your changes are still here.".into(), cx);
        }
        self.show_notice("Back as it was saved: ⌘Z brings your changes back.".into(), cx);
    }

    /// Organize Imports: the language server's own (sorted, the unused ones gone), for the
    /// whole file, as its quick fixes are applied.
    fn organize_imports(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let e = editor.read(cx);
        let Some(path) = e.path().map(Path::to_path_buf) else { return };
        if !self.lsp.read(cx).serves(&path) {
            return self.show_notice("No language server for this file: nothing to organize imports with.".into(), cx);
        }
        let last = e.buffer.len_lines().saturating_sub(1);
        let end = lsp_types::Position::new(last as u32, e.buffer.line_text(last).encode_utf16().count() as u32);
        let range = lsp_types::Range::new(lsp_types::Position::new(0, 0), end);
        let only = Some(vec![lsp_types::CodeActionKind::SOURCE_ORGANIZE_IMPORTS]);
        let request = self.lsp.read(cx).code_actions_of(&path, range, Vec::new(), only);
        cx.spawn(async move |this, cx| {
            let actions = request.await;
            this.update(cx, |this, cx| match organize_action(actions) {
                Some(action) => this.run_fix(path, action, cx),
                None => this.show_notice("The language server has no imports to organize here.".into(), cx),
            })
            .ok();
        })
        .detach();
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
                        crate::system_clipboard::write(cx, gpui::ClipboardItem::new_string(link));
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
        let Some(clipboard) = crate::system_clipboard::read(cx).and_then(|item| item.text()) else {
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
                // Still possible (another move, dropped together, may have taken the name since)?
                // If not, nothing that names it changes either.
                // (A new case for the same name, on a disk that ignores case, is the same file.)
                if !from.exists() || (to != from && to.exists() && !crate::fs_ops::same_file(&from, &to)) {
                    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    return this.show_notice(format!("Couldn't move it: {name} is taken there"), cx);
                }
                // Moved first: if that fails (no room, no right, another disk), nothing that
                // names it has been changed to a place it isn't.
                let result = this.tree.update(cx, |tree, cx| tree.finish_rename(&from, &to, cx));
                if let Err(error) = result {
                    return this.show_notice(error, cx);
                }
                // Its tabs moved with it now, not once the tree's word of it arrives (after
                // this): edits to the moved file itself go to its open tab, not the disk.
                this.paths_renamed(&from, &to, cx);
                // What every server asked for, as one, at the files where they are now: the
                // files changed and those that couldn't be.
                let changed = (!edits.is_empty()).then(|| {
                    edits
                        .into_iter()
                        .map(|edit| this.apply_edit_after_move(edit, Some((from.as_path(), to.as_path())), cx))
                        .fold((0, 0, Vec::new()), |(places, files, mut failed), (p, f, mut x)| {
                            failed.append(&mut x);
                            (places + p, files + f, failed)
                        })
                });
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
            // The next one asked for (several dropped together), from what this left.
            this.update(cx, |this, cx| {
                this.renaming = false;
                this.next_rename(cx);
            })
            .ok();
        })
        .detach();
    }

    /// A rename or move asked for: done after those before it, each working out what
    /// names it from the files as the last one left them (two at once read the same
    /// text, and the second's changes landed in the wrong places).
    fn queue_rename(&mut self, from: PathBuf, to: PathBuf, cx: &mut Context<Self>) {
        self.renames.push_back((from, to));
        self.next_rename(cx);
    }

    fn next_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming {
            return;
        }
        if let Some((from, to)) = self.renames.pop_front() {
            self.renaming = true;
            self.rename_file(from, to, cx);
        }
    }

    /// Files moved outside Null (`mv`, `git mv`, another tool): an open file that's gone,
    /// and one with the same text that appeared with it, is the same file moved. Its tab
    /// follows, with the others from its folder when the folder moved.
    fn follow_moves(&mut self, changed: &[PathBuf], cx: &mut Context<Self>) {
        // Only for a tab whose file is gone; among a few hundred changes at most (each one
        // looked at on disk: a checkout of thousands isn't searched through).
        if !self.tabs.iter().any(|t| t.editor.read(cx).missing) {
            return;
        }
        let changed = &changed[..changed.len().min(300)];
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
        // Nothing moved: the files' selection (what's picked in them) stays as it is.
        if moves.is_empty() {
            return;
        }
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
        self.open_file_as(path, side, false, window, cx);
    }

    /// A file looked at in passing (one click in the files): in the passing tab, which it
    /// replaces, unless it's open already.
    fn open_file_passing(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let passing = cx.global::<Settings>().preview_tabs;
        self.open_file_as(path, None, passing, window, cx);
    }

    /// Opens a file (see `open_file_on`). Opened on purpose (not `passing`), it's kept: a
    /// passing tab showing it stays.
    fn open_file_as(
        &mut self,
        path: PathBuf,
        side: Option<usize>,
        passing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Open on both sides: the copy on the side asked for (or being worked in).
        let wanted = side.unwrap_or_else(|| self.focused_side());
        let copies: Vec<usize> =
            (0..self.tabs.len()).filter(|&i| self.tabs[i].editor.read(cx).path() == Some(path.as_path())).collect();
        if let Some(&ix) = copies.iter().find(|&&i| self.tabs[i].side == wanted).or(copies.first()) {
            if !passing {
                self.tabs[ix].passing = false;
            }
            self.activate(ix, window, cx);
            return;
        }
        // The one passing on that side gives way (it was never edited, or it'd be kept).
        let gives_way = passing
            .then(|| {
                self.tabs
                    .iter()
                    .find(|t| t.passing && t.side == wanted && !t.pinned && !t.editor.read(cx).buffer.is_dirty())
            })
            .flatten()
            .map(|t| t.editor.clone());
        let lsp = self.lsp.clone();
        let editor = cx.new(|cx| Editor::open(path, Some(lsp), cx));
        self.add_tab_on(editor.clone(), side, window, cx);
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.editor == editor) {
            tab.passing = passing;
        }
        if let Some(old) = gives_way.and_then(|old| self.tabs.iter().position(|t| t.editor == old)) {
            self.remove_tab(old, window, cx);
            // Only looked at: not for Reopen Closed Tab.
            self.recently_closed.pop();
        }
    }

    /// The tab showing `editor` kept: no longer passing.
    fn keep_tab(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| &t.editor == editor && t.passing) {
            tab.passing = false;
            cx.notify();
        }
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
        // (A Markdown file's lines are its preview's; a notebook's JSON lines aren't.)
        if source.read(cx).reading || !source.read(cx).is_markdown() {
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
        // A new split (⌃⌥⌘→): side by side, however the last one was.
        if !self.is_split() {
            self.stacked = false;
        }
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
                    // Edited: worth keeping, and its copy on the other side too.
                    this.keep_tab(editor, cx);
                    for twin in this.twins_of(editor, cx) {
                        this.keep_tab(&twin, cx);
                    }
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
                EditorEvent::ReadFromDisk => {
                    // Its other copy has what's on disk too (reloaded itself, or given it now):
                    // the read isn't replayed on it as an edit (that doubled its end).
                    let (text, saved, on_disk) = {
                        let e = editor.read(cx);
                        (e.buffer.rope().clone(), !e.buffer.is_dirty(), e.on_disk)
                    };
                    for twin in this.twins_of(editor, cx) {
                        if *twin.read(cx).buffer.rope() != text {
                            twin.update(cx, |t, cx| t.apply_twin_edits(None, &text, saved, cx));
                        }
                        // What's on disk, as this copy has it: no change on disk left to ask about.
                        if saved {
                            twin.update(cx, |t, _| {
                                t.on_disk = on_disk;
                                t.disk_changed = false;
                            });
                        }
                        this.twin_seen.insert(twin.entity_id(), twin.read(cx).buffer.revision());
                    }
                    this.twin_seen.insert(editor.entity_id(), editor.read(cx).buffer.revision());
                }
                EditorEvent::Saved => {
                    if let Some(path) = editor.read(cx).path().map(Path::to_path_buf) {
                        let text = editor.read(cx).buffer.to_string();
                        this.tests_saved(path, text, cx);
                    }
                    // Saved while an AI task works: that file's change is yours too.
                    // (Not the saves that start it: those are in the copy it's compared with.)
                    if let Some(run) = this.ai_task.as_mut().filter(|r| matches!(r.state, TaskState::Running(_)))
                        && let Some(path) = editor.read(cx).path()
                    {
                        run.yours.insert(path.to_path_buf());
                    }
                    this.schedule_backup(cx);
                    this.refresh_git_status(cx);
                    // Saving (and tidying) one copy saves the other: same text, same file.
                    this.sync_twins(editor, cx);
                    let (text, on_disk) = {
                        let e = editor.read(cx);
                        (e.buffer.rope().clone(), e.on_disk)
                    };
                    for twin in this.twins_of(editor, cx) {
                        twin.update(cx, |twin, cx| {
                            // Saved only if it reads the same: what's on disk now is this.
                            if *twin.buffer.rope() == text {
                                twin.buffer.mark_saved();
                                twin.on_disk = on_disk;
                                twin.disk_changed = false;
                            }
                            cx.notify();
                        });
                    }
                    this.share_unsaved(editor, cx);
                    this.refresh_title(window, cx);
                    // A theme of yours, saved: shown as it is now, if it's the one in use.
                    let own = crate::theme::own::folder();
                    let theme_file = editor
                        .read(cx)
                        .path()
                        .filter(|p| p.parent() == own.as_deref())
                        .map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
                    if let Some(name) = theme_file {
                        this.theme_saved(&name, cx);
                    }
                    let root = this.tree.read(cx).root().to_path_buf();
                    let path = editor.read(cx).path();
                    if path == Some(crate::settings::project_file(&root).as_path())
                        || path == Some(crate::settings::vscode_file(&root).as_path())
                    {
                        settings::use_project(&root, cx);
                        if settings::project_unreadable() {
                            this.show_notice(
                                ".null/settings.json can't be read: it's left as it is until it's put right".into(),
                                cx,
                            );
                        }
                    }
                    if editor.read(cx).path().is_some_and(|p| Some(p) == Settings::path().as_deref()) {
                        settings::reload(cx);
                        // Not readable as written: said, and nothing saved over it meanwhile.
                        if let Some(why) = Settings::unreadable() {
                            this.show_notice(
                                format!("settings.json can't be read ({why}): nothing changes until it's put right"),
                                cx,
                            );
                        }
                        // A shortcut of yours that couldn't be understood: say which.
                        if let Some(problem) =
                            cx.try_global::<crate::user_keys::KeyProblems>().and_then(|p| p.0.first().cloned())
                        {
                            this.show_notice(format!("Shortcut {problem}."), cx);
                        }
                    }
                    cx.notify();
                }
                EditorEvent::Reread { encoding, became_text } => {
                    let (encoding, became_text) = (*encoding, *became_text);
                    if became_text {
                        let lsp = this.lsp.clone();
                        editor.update(cx, |e, cx| e.connect_lsp(lsp, cx));
                    }
                    // Its other copy reads it the same; neither passes the change back.
                    for twin in this.twins_of(editor, cx) {
                        twin.update(cx, |t, cx| t.reread_as(encoding, cx)).ok();
                        this.twin_seen.insert(twin.entity_id(), twin.read(cx).buffer.revision());
                    }
                    this.twin_seen.insert(editor.entity_id(), editor.read(cx).buffer.revision());
                }
                EditorEvent::EncodingChanged(encoding) => {
                    for twin in this.twins_of(editor, cx) {
                        twin.update(cx, |t, _| t.encoding = *encoding);
                    }
                }
                EditorEvent::NeedsPath => this.ask_where_to_save(editor.clone(), window, cx),
                EditorEvent::VimCommandLine => this.open_palette(PaletteKind::Ex, window, cx),
                EditorEvent::SavedToClose => {
                    this.confirm_unsaved(CloseAction::CloseTabs(vec![editor.clone()]), window, cx);
                }
                EditorEvent::RunCommand(command) => this.run_in_terminal(command.clone(), None, window, cx),
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
                    this.run_fix(path, (**fix).clone(), cx);
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
                EditorEvent::BookmarksChanged => {
                    // The other copy of the file, on the other side, has the same bookmarks.
                    let (path, lines) = {
                        let editor = editor.read(cx);
                        (editor.path().map(Path::to_path_buf), editor.bookmarks.clone())
                    };
                    for tab in &this.tabs {
                        if tab.editor != *editor && path.is_some() && tab.editor.read(cx).path() == path.as_deref() {
                            tab.editor.update(cx, |twin, cx| twin.set_bookmarks(lines.clone(), cx));
                        }
                    }
                    this.schedule_session_save(cx);
                }
                EditorEvent::Open(path) => {
                    if let Some(place) = editor
                        .read(cx)
                        .path()
                        .map(|p| Place { path: p.to_path_buf(), point: editor.read(cx).caret_point() })
                    {
                        this.remember_place(Some(place));
                    }
                    this.open_file(path.clone(), window, cx);
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
        self.tabs.insert(
            ix,
            Tab { editor: editor.clone(), side, pinned: false, passing: false, _subscriptions: subscriptions },
        );
        // Among pinned tabs, it goes after them: shown wherever it ended up.
        self.keep_pinned_first();
        let ix = self.tabs.iter().position(|t| t.editor == editor).unwrap_or(ix);
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
        // Another project's tests: looked for again, if they show.
        self.tests = None;
        self.tests_task = None;
        self.test_status.clear();
        self.tests_running = None;
        self.refresh_git(cx);
        self.watch(path.clone(), cx);
        self.tree.update(cx, |tree, cx| tree.set_root(path.clone(), cx));
        self.ignore_rules = crate::project_index::ignore_rules(&path);
        self.reindex_pending.clear();
        self.build_index(cx);
        if self.sidebar_tests {
            self.find_tests(cx);
        }
        self.python_env = crate::python_env::find(&path);
        self.python_env_here = None;
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
        // Used now (unless only passed on the way, ⌃ held after ⌃Tab).
        if self.switching.is_none() {
            let id = tab.editor.entity_id();
            self.used.retain(|u| *u != id);
            self.used.insert(0, id);
        }
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
        let had_focus = tab.editor.read(cx).has_keys(window, cx);
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

    /// Moves the current tab to the other side (⌃⌘→ / ⌃⌘←, or below and above), opening the
    /// split when needed. `stacked`, going to the second side: whether it goes below the
    /// first, or right of it (already there, the sides turn).
    fn move_tab_to(&mut self, to: usize, stacked: Option<bool>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else { return };
        let from = self.tabs[ix].side;
        if from == to {
            if let Some(stacked) = stacked.filter(|s| to == 1 && *s != self.stacked) {
                self.stacked = stacked;
                self.schedule_session_save(cx);
                cx.notify();
            }
            return;
        }
        if self.tabs.len() < 2 {
            return self.show_notice("Open another file to see two side by side".into(), cx);
        }
        if let Some(stacked) = stacked.filter(|_| to == 1) {
            self.stacked = stacked;
        }
        self.tabs[ix].side = to;
        // Moved on purpose: kept.
        self.tabs[ix].passing = false;
        let left_behind = self.side_tabs(from);
        let neighbour = left_behind.iter().find(|&&i| i >= ix).or(left_behind.last());
        self.shown[from] = neighbour.map(|&i| self.tabs[i].editor.clone());
        if self.side_tabs(0).is_empty() {
            for t in &mut self.tabs {
                t.side = 0;
            }
            self.shown = [None, None];
        }
        let moved = self.tabs[ix].editor.clone();
        self.keep_pinned_first();
        let ix = self.tabs.iter().position(|t| t.editor == moved).unwrap_or(ix);
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
        // Dropped among the pinned ones: it goes after them (they stay first).
        self.keep_pinned_first();
        let at = self.tabs.iter().position(|t| &t.editor == editor).unwrap_or(at);
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
        // Quitting, closing the window, opening another project: unsaved work is kept for
        // next time rather than asked about (closing a tab still asks).
        if cx.global::<Settings>().keep_unsaved && !matches!(action, CloseAction::CloseTabs(_)) {
            let root = self.tree.read(cx).root().to_path_buf();
            crate::session::save_backups(&root, &Self::backups_of(self.unsaved_files(cx)));
            self.keeping_unsaved = true;
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
                    // Quitting called off: the windows that were done asking ask again next time.
                    if matches!(action, CloseAction::Quit) {
                        cx.defer(crate::quit_cancelled);
                    }
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
        let name = editor.read(cx).suggested_name();
        let answer = cx.prompt_for_new_path(&root, Some(&name));
        let lsp = self.lsp.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = answer.await else {
                if matches!(action, CloseAction::Quit) {
                    cx.update(|_, cx| cx.defer(crate::quit_cancelled)).ok();
                }
                return;
            };
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
        // Every unsaved change was saved or let go: no backup to bring back (unless they
        // were kept for next time).
        if !matches!(action, CloseAction::CloseTabs(_)) {
            self.backup_task = None;
            if !std::mem::take(&mut self.keeping_unsaved) {
                crate::session::save_backups(self.tree.read(cx).root(), &[]);
            }
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
        let editors = self.unpinned().collect();
        self.confirm_unsaved(CloseAction::CloseTabs(editors), window, cx);
    }

    fn close_other_tabs(&mut self, _: &CloseOtherTabs, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.active_editor().cloned();
        let editors = self.unpinned().filter(|e| Some(e) != active.as_ref()).collect();
        self.confirm_unsaved(CloseAction::CloseTabs(editors), window, cx);
    }

    /// The tabs that aren't pinned: what closing "all" or "the others" means.
    fn unpinned(&self) -> impl Iterator<Item = Entity<Editor>> + '_ {
        self.tabs.iter().filter(|t| !t.pinned).map(|t| t.editor.clone())
    }

    /// Pins a tab (to the front of its side, after the other pinned ones) or unpins it
    /// (just after them).
    fn set_pinned(&mut self, editor: &Entity<Editor>, pinned: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.tabs.iter().position(|t| &t.editor == editor) else { return };
        if self.tabs[ix].pinned == pinned {
            return;
        }
        let active = self.active.map(|a| self.tabs[a].editor.clone());
        let mut tab = self.tabs.remove(ix);
        tab.pinned = pinned;
        // Pinned: kept, surely.
        tab.passing &= !pinned;
        let side = tab.side;
        // After the last pinned tab of that side (or where the side starts).
        let at = self
            .tabs
            .iter()
            .rposition(|t| t.side == side && t.pinned)
            .map(|i| i + 1)
            .or_else(|| self.tabs.iter().position(|t| t.side == side))
            .unwrap_or(self.tabs.len());
        self.tabs.insert(at, tab);
        self.active = active.and_then(|a| self.tabs.iter().position(|t| t.editor == a));
        self.schedule_session_save(cx);
        cx.notify();
    }

    /// Pinned tabs first on each side, the order otherwise kept (after a tab is added,
    /// moved or dropped among them). Each side keeps its own places in the list.
    fn keep_pinned_first(&mut self) {
        let active = self.active.map(|a| self.tabs[a].editor.clone());
        let mut slots: Vec<Option<Tab>> = std::mem::take(&mut self.tabs).into_iter().map(Some).collect();
        for side in 0..2 {
            let places: Vec<usize> =
                (0..slots.len()).filter(|&i| slots[i].as_ref().is_some_and(|t| t.side == side)).collect();
            let mut tabs: Vec<Tab> = places.iter().filter_map(|&i| slots[i].take()).collect();
            tabs.sort_by_key(|t| !t.pinned);
            for (place, tab) in places.into_iter().zip(tabs) {
                slots[place] = Some(tab);
            }
        }
        self.tabs = slots.into_iter().flatten().collect();
        self.active = active.and_then(|a| self.tabs.iter().position(|t| t.editor == a));
    }

    fn reopen_closed_tab(&mut self, _: &ReopenClosedTab, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(path) = self.recently_closed.pop() {
            if path.exists() {
                return self.open_file(path, window, cx);
            }
        }
    }

    /// What the language servers said about themselves, in a tab of its own (not a file):
    /// the one already open, brought up to date.
    fn show_server_log(&mut self, _: &ShowServerLog, window: &mut Window, cx: &mut Context<Self>) {
        const NAME: &str = "Language server log";
        let log = crate::server_log::text();
        let body = if log.is_empty() { "Nothing yet: no language server has started.".to_string() } else { log };
        let text = format!("{body}\n");
        let open = self.tabs.iter().position(|t| t.editor.read(cx).untitled_name.as_deref() == Some(NAME));
        let editor = match open {
            Some(ix) => {
                let editor = self.tabs[ix].editor.clone();
                editor.update(cx, |e, cx| {
                    e.restore_unsaved(&text, cx);
                    e.buffer.mark_saved();
                });
                self.activate(ix, window, cx);
                editor
            }
            None => {
                let editor = cx.new(|cx| {
                    let mut editor = Editor::new(crate::buffer::Buffer::from_text(&text), None, cx);
                    editor.untitled_name = Some(NAME.into());
                    editor
                });
                self.add_tab(editor.clone(), window, cx);
                editor
            }
        };
        // At its end: the latest.
        editor.update(cx, |e, cx| {
            let last = e.buffer.len_lines().saturating_sub(1);
            e.set_caret_point((last, 0), cx);
        });
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
                    Ok(1) => {
                        editor.revert_to_disk(cx);
                    }
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
        // Untitled: a name from what it holds (`Trip to Lyon.md`, `untitled.py`).
        let name = match current.as_ref().and_then(|p| p.file_name()) {
            Some(name) => name.to_string_lossy().into_owned(),
            None => editor.read(cx).suggested_name(),
        };
        let answer = cx.prompt_for_new_path(&dir, Some(&name));
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

    /// ⌃Tab: the tab used before this one; again with ⌃ still held, the one before that,
    /// and so on (⌃⇧Tab back). Letting go of ⌃ stays there.
    fn switch_tab(&mut self, back: bool, held: bool, window: &mut Window, cx: &mut Context<Self>) {
        let open: Vec<gpui::EntityId> = self.tabs.iter().map(|t| t.editor.entity_id()).collect();
        self.used.retain(|id| open.contains(id));
        // Tabs never shown yet (opened with others, from the last session) come last.
        for id in open {
            if !self.used.contains(&id) {
                self.used.push(id);
            }
        }
        if self.used.len() < 2 {
            return;
        }
        // Still switching, but another tab was shown meanwhile (a click): start from it.
        let active = self.active_editor().map(|e| e.entity_id());
        if self.switching.is_some_and(|at| self.used.get(at).copied() != active) {
            self.settle_switch();
            if let Some(id) = active {
                self.used.retain(|u| *u != id);
                self.used.insert(0, id);
            }
        }
        let at = self.switching.unwrap_or(0) as isize;
        let next = (at + if back { -1 } else { 1 }).rem_euclid(self.used.len() as isize) as usize;
        self.switching = Some(next);
        let id = self.used[next];
        if let Some(ix) = self.tabs.iter().position(|t| t.editor.entity_id() == id) {
            self.activate(ix, window, cx);
        }
        // Without ⌃ held (from a menu, or a key of your own): that's the one.
        if !held {
            self.settle_switch();
        }
    }

    /// ⌃ let go after ⌃Tab: the tab reached is now the last used.
    fn settle_switch(&mut self) {
        if let Some(at) = self.switching.take() {
            let id = self.used.remove(at);
            self.used.insert(0, id);
        }
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
        let theme_label = |name: ThemeName| {
            format!("{} Theme{}", name.label(), current(settings.theme == name && settings.own_theme.is_none()))
        };
        let toggle = |on: bool, stop: &str, start: &str| if on { stop.to_string() } else { start.to_string() };
        // The file at hand's own settings, where its language has them.
        let language = self.active_language(cx);
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
            (View, toggle(settings.code_lens, "Hide Code Lens", "Show Code Lens"), Box::new(ToggleCodeLens)),
            (View, "Show Language Server Log".into(), Box::new(ShowServerLog)),
            (
                View,
                toggle(settings.bracket_colours, "Plain Brackets", "Colour Bracket Pairs"),
                Box::new(ToggleBracketColours),
            ),
            (
                View,
                toggle(settings.problems_at_line_ends, "Problems Only on the Caret's Line", "Problems at Line Ends"),
                Box::new(ToggleProblemsAtLineEnds),
            ),
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
            (Edit, "Toggle Bookmark".into(), Box::new(crate::editor::ToggleBookmark)),
            (Go, "Next Bookmark".into(), Box::new(crate::editor::NextBookmark)),
            (Go, "Previous Bookmark".into(), Box::new(crate::editor::PreviousBookmark)),
            (Go, "Bookmarks…".into(), Box::new(ShowBookmarks)),
            (App, "Edit Snippets…".into(), Box::new(EditSnippets)),
            (View, "Run Selection in Terminal".into(), Box::new(RunSelectionInTerminal)),
            (Edit, "Paste from History…".into(), Box::new(PasteFromHistory)),
            (Edit, "Copy as Code Block".into(), Box::new(CopyAsCodeBlock)),
            (Edit, "Organize Imports".into(), Box::new(OrganizeImports)),
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
            (View, "Show Outline".into(), Box::new(ShowOutline)),
            (View, "Show Tests".into(), Box::new(ShowTests)),
            (
                View,
                toggle(settings.word_wrap_for(language), "Stop Wrapping Lines", "Wrap Lines"),
                Box::new(ToggleWordWrap),
            ),
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
            (Appearance, "New Theme of Your Own…".into(), Box::new(NewOwnTheme)),
            (Appearance, "Bigger Text".into(), Box::new(IncreaseFontSize)),
            (Appearance, "Compact Line Spacing".into(), Box::new(CompactLineSpacing)),
            (Appearance, "Normal Line Spacing".into(), Box::new(NormalLineSpacing)),
            (Appearance, "Relaxed Line Spacing".into(), Box::new(RelaxedLineSpacing)),
            (Appearance, "Smaller Text".into(), Box::new(DecreaseFontSize)),
            (Appearance, "Actual Size".into(), Box::new(ResetFontSize)),
            (
                Edit,
                toggle(settings.autocomplete_for(language), "Turn Off Autocomplete", "Turn On Autocomplete"),
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
            (
                Go,
                toggle(
                    self.debugger.read(cx).stop_on_errors,
                    "Don't Stop on Errors While Debugging",
                    "Stop on Errors While Debugging",
                ),
                Box::new(ToggleStopOnErrors),
            ),
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
            (View, "Split Terminal".into(), Box::new(SplitTerminal)),
            (App, "Edit Settings as JSON".into(), Box::new(OpenSettingsFile)),
            (App, "Edit Settings for This Project".into(), Box::new(OpenProjectSettings)),
        ];
        if self.active.is_some() {
            commands.extend([
                (File, "Save".into(), Box::new(Save) as Box<dyn Action>),
                (File, "Save As…".into(), Box::new(SaveAs)),
                (File, "Revert to Saved".into(), Box::new(RevertToSaved)),
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
                (Edit, "Remove Invisible Characters".into(), Box::new(crate::editor::RemoveInvisibleCharacters)),
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
                (Edit, "Increment Number or Flip Value".into(), Box::new(crate::editor::Increment)),
                (Edit, "Decrement Number or Flip Value".into(), Box::new(crate::editor::Decrement)),
                (Edit, "Expand Selection".into(), Box::new(crate::editor::ExpandSelection)),
                (Edit, "Shrink Selection".into(), Box::new(crate::editor::ShrinkSelection)),
                (Go, "Go to Matching Bracket".into(), Box::new(crate::editor::GoToMatchingBracket)),
                (Lines, "Indent with Tabs".into(), Box::new(crate::editor::IndentWithTabs)),
                (Lines, "Indent with 2 Spaces".into(), Box::new(crate::editor::IndentWith2Spaces)),
                (Lines, "Indent with 4 Spaces".into(), Box::new(crate::editor::IndentWith4Spaces)),
                (Lines, "Use LF Line Endings (macOS, Linux)".into(), Box::new(crate::editor::UseLfLineEndings)),
                (Lines, "Use CRLF Line Endings (Windows)".into(), Box::new(crate::editor::UseCrlfLineEndings)),
                (Lines, "Fold".into(), Box::new(crate::editor::Fold)),
                (Lines, "Unfold".into(), Box::new(crate::editor::Unfold)),
                (Lines, "Fold All".into(), Box::new(crate::editor::FoldAll)),
                (Lines, "Fold Level 1 (the outermost blocks)".into(), Box::new(crate::editor::FoldLevel1)),
                (Lines, "Fold Level 2".into(), Box::new(crate::editor::FoldLevel2)),
                (Lines, "Fold Level 3".into(), Box::new(crate::editor::FoldLevel3)),
                (Lines, "Unfold All".into(), Box::new(crate::editor::UnfoldAll)),
                (Cursors, "Add Next Occurrence".into(), Box::new(crate::editor::AddNextOccurrence)),
                (Cursors, "Skip This Occurrence".into(), Box::new(crate::editor::SkipOccurrence)),
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
                (Go, "Show Callees (What This Calls)".into(), Box::new(crate::editor::ShowCallees)),
                (Go, "Go to Implementation".into(), Box::new(crate::editor::GoToImplementation)),
                (Go, "Go to Type Definition".into(), Box::new(crate::editor::GoToTypeDefinition)),
                (Go, "Show Problems".into(), Box::new(ShowProblems)),
                (Edit, "Rename Symbol".into(), Box::new(crate::editor::RenameSymbol)),
                (Edit, "Quick Fix…".into(), Box::new(crate::editor::QuickFix)),
                (Edit, "Format Document".into(), Box::new(crate::editor::FormatDocument)),
                (Edit, "Format Selection".into(), Box::new(crate::editor::FormatSelection)),
                (
                    Edit,
                    toggle(settings.format_on_save_for(language), "Stop Formatting on Save", "Format on Save"),
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
                (Go, "Last Used Tab".into(), Box::new(SwitchTab)),
                (View, "Move Tab to the Right Side".into(), Box::new(MoveTabRight)),
                (View, "Move Tab to the Left Side".into(), Box::new(MoveTabLeft)),
                (View, "Move Tab Below".into(), Box::new(MoveTabDown)),
                (View, "Move Tab Above".into(), Box::new(MoveTabUp)),
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
        // Rust: what a macro expands to (rust-analyzer).
        if self.active_editor().is_some_and(|e| e.read(cx).language_name() == "Rust") {
            commands.push((Go, "Expand Macro".into(), Box::new(crate::editor::ExpandMacro)));
            commands.push((Go, "Open Documentation".into(), Box::new(crate::editor::OpenDocumentation)));
            commands.push((Go, "Go to Parent Module".into(), Box::new(crate::editor::GoToParentModule)));
            commands.push((Go, "Open Cargo.toml".into(), Box::new(crate::editor::OpenCargoToml)));
        }
        // C, C++, Objective-C: from a source to its header and back.
        if self.active_editor().and_then(|e| e.read(cx).path()).is_some_and(crate::editor::has_counterpart) {
            commands.push((Go, "Switch Header/Source".into(), Box::new(crate::editor::SwitchSourceHeader)));
        }
        // Markdown formatted, code in its colours: to paste into Mail, Keynote, Pages.
        if let Some(editor) = self.active_editor() {
            let label = if editor.read(cx).is_markdown() { "Copy as Rich Text" } else { "Copy with Colours" };
            commands.push((Edit, label.into(), Box::new(CopyAsRichText)));
        }
        // Markdown: a table of contents of its headings; the document as a page.
        if self.active_editor().is_some_and(|e| e.read(cx).is_markdown()) {
            commands.push((Edit, "Insert Table of Contents".into(), Box::new(crate::editor::InsertTableOfContents)));
            commands.push((Edit, "Insert Footnote".into(), Box::new(crate::editor::InsertFootnote)));
            commands.push((Edit, "Toggle Task".into(), Box::new(crate::editor::ToggleTask)));
            commands.push((File, "Export as HTML".into(), Box::new(ExportHtml)));
        }
        // A page or a picture: open it in the browser.
        if self.active_editor().and_then(|e| e.read(cx).path()).is_some_and(crate::file_tree::opens_in_browser) {
            commands.push((File, "Open in Browser".into(), Box::new(OpenInBrowser)));
        }
        // Themes of your own, beside Null's.
        for name in crate::theme::own::names() {
            let label = format!("{name} Theme{}", current(settings.own_theme.as_deref() == Some(name.as_str())));
            commands.push((Appearance, label, Box::new(UseOwnTheme { name })));
        }
        // The file's encoding: read again as another, or written in another.
        if let Some(editor) = self.active_editor().filter(|e| e.read(cx).path().is_some()) {
            let now = editor.read(cx).encoding;
            for encoding in crate::encoding::Encoding::ALL {
                let label = format!("Encoding: Reopen as {}", encoding.label());
                commands.push((File, label, Box::new(crate::editor::ReopenWithEncoding { encoding })));
            }
            for encoding in crate::encoding::Encoding::ALL {
                let label = format!("Encoding: Save as {}{}", encoding.label(), current(encoding == now));
                commands.push((File, label, Box::new(crate::editor::SaveWithEncoding { encoding })));
            }
        }
        // The file's language, to put it in another (a script without an extension).
        if let Some(editor) = self.active_editor() {
            let now = editor.read(cx).language_name();
            // The one it's in first: what the language in the status bar opens on.
            let mut names: Vec<&'static str> = crate::editor::language_names().into_iter().collect();
            names.sort_by_key(|name| *name != now);
            for name in names {
                let label = format!("Language: {name}{}", current(name == now));
                commands.push((View, label, Box::new(crate::editor::SetLanguage { name })));
            }
        }
        if !self.terminals.is_empty() {
            commands.push((View, "Rename Terminal".into(), Box::new(RenameTerminal)));
        }
        if let Some(tab) = self.active.and_then(|a| self.tabs.get(a)) {
            let label = if tab.pinned { "Unpin Tab" } else { "Pin Tab" };
            commands.push((View, label.into(), Box::new(TogglePinTab)));
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
        // A list replacing another: what the last one's rows stood for is forgotten (its
        // own is set after this, by whoever opens it).
        self.clipboard_list = None;
        self.history = None;
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
            language_here: self.active_language(cx),
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
            PaletteEvent::RunCommand(command, task) => {
                let (command, task) = (command.clone(), task.clone());
                this.close_palette(window, cx);
                this.run_in_terminal(command, task, window, cx);
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
            PaletteEvent::Ex(text) => {
                let text = text.clone();
                this.close_palette(window, cx);
                this.run_ex(&text, window, cx);
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
                let clipboard = this.clipboard_list.take();
                this.close_palette(window, cx);
                // A copy picked: on the clipboard again, and pasted where the caret is.
                if let Some(item) = clipboard.and_then(|list| list.get(position.line as usize).cloned()) {
                    crate::system_clipboard::write(cx, item);
                    if let Some(editor) = this.active_editor().cloned() {
                        window.focus(&editor.focus_handle(cx));
                        window.dispatch_action(Box::new(crate::editor::Paste), cx);
                    }
                    return;
                }
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
                // Copies aren't places in a file.
                if this.clipboard_list.is_some() {
                    return;
                }
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
        self.clipboard_list = None;
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

    /// Bookmarks…: the lines bookmarked in every open file, to go to.
    fn show_bookmarks(&mut self, _: &ShowBookmarks, window: &mut Window, cx: &mut Context<Self>) {
        let mut rows: Vec<crate::palette::Location> = Vec::new();
        for tab in &self.tabs {
            let editor = tab.editor.read(cx);
            let Some(path) = editor.path() else { continue };
            // A file open on both sides is listed once.
            if rows.iter().any(|r| r.path == path) {
                continue;
            }
            rows.extend(editor.bookmarks.iter().map(|&line| crate::palette::Location {
                path: path.to_path_buf(),
                position: lsp_types::Position::new(line as u32, 0),
                text: editor.buffer.line_text(line).trim().to_string(),
                kind: crate::palette::LocationKind::Reference,
            }));
        }
        rows.sort_by(|a, b| (&a.path, a.position.line).cmp(&(&b.path, b.position.line)));
        if rows.is_empty() {
            self.show_notice("No bookmarks yet: ⌘F2 marks the caret's line".into(), cx);
            return;
        }
        self.open_locations("Bookmarks".into(), rows, window, cx);
    }

    /// Edit Snippets…: the snippets file for the current file's language, made with a short
    /// how-to the first time.
    fn edit_snippets(&mut self, _: &EditSnippets, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.active_editor().map(|e| e.read(cx));
        let (id, name) = match editor.and_then(|e| e.path().map(|p| (p, e.language()))) {
            Some((path, language)) => {
                let name = language.map(|l| l.name);
                (crate::snippets::language_id(name, path), name.unwrap_or("plain text"))
            }
            None => ("plaintext", "plain text"),
        };
        let Some(folder) = crate::snippets::folder() else { return };
        let file = folder.join(format!("{id}.json"));
        if !file.exists() {
            let made =
                std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&file, crate::snippets::new_file(name)));
            if let Err(error) = made {
                return self.show_notice(format!("Couldn't make {}: {error}", file.display()), cx);
            }
        }
        self.open_file(file, window, cx);
    }

    /// Project search, in folder `dir` only (from the files' menu).
    fn find_in_folder(&mut self, dir: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        let folder = dir.strip_prefix(&root).unwrap_or(dir).to_string_lossy().replace('\\', "/");
        self.sidebar_search.set(true, SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        settings::update(cx, |s| s.sidebar_visible = true);
        self.project_search.update(cx, |search, cx| search.search_in_folder(&folder, window, cx));
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
        self.ai_task = Some(AiTaskRun {
            title,
            state: TaskState::Starting,
            child,
            stop,
            yours: HashSet::new(),
            _task: Some(task),
        });
        cx.notify();
    }

    fn task_finished(
        &mut self,
        result: Result<String, String>,
        changes: Vec<crate::ai_task::FileChange>,
        cx: &mut Context<Self>,
    ) {
        // Saved by you meanwhile: marked, so undoing all leaves them to you.
        let mut changes = changes;
        if let Some(run) = &self.ai_task {
            for change in &mut changes {
                change.yours_too = run.yours.contains(&change.path);
            }
        }
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
        let mut left = Vec::new();
        for change in &changes {
            let open = self.tabs.iter().find(|t| t.editor.read(cx).path() == Some(change.path.as_path()));
            // Yours too (saved during the task), or changed since it ended: not the task's
            // to take back. Left for you to review.
            let now = match open {
                Some(tab) => Some(tab.editor.read(cx).buffer.to_string()),
                None => std::fs::read_to_string(&change.path).ok(),
            };
            let as_left = match change.kind {
                crate::ai_task::ChangeKind::Deleted => !change.path.exists(),
                _ => now.as_deref() == Some(change.after.as_str()),
            };
            if change.yours_too || !as_left {
                left.push(change.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()));
                continue;
            }
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
        let message = match (failed.is_empty(), left.is_empty()) {
            (true, true) => "Every file is back as it was before the task.".to_string(),
            (true, false) => format!("Undone, but left as they are (edited by you too): {}", left.join(", ")),
            (false, _) => format!("Couldn't undo everything: {}", failed.join("; ")),
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
                    // Not one that would come back changed where nothing was replaced.
                    let Ok((text, encoding)) = crate::encoding::read_whole(path) else {
                        failed.push(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                        continue;
                    };
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
        self.sidebar_outline = false;
        self.sidebar_tests = false;
        self.sidebar_search.set(false, SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        settings::update(cx, |s| s.sidebar_visible = true);
        self.leave_hidden_focus(window, cx);
        cx.notify();
    }

    fn show_outline(&mut self, _: &ShowOutline, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_outline = true;
        self.sidebar_tests = false;
        self.outline_followed = None;
        self.sidebar_search.set(false, SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        settings::update(cx, |s| s.sidebar_visible = true);
        self.leave_hidden_focus(window, cx);
        cx.notify();
    }

    /// The project's tests in the sidebar, looked for again (they may have changed).
    fn show_tests(&mut self, _: &ShowTests, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_tests = true;
        self.sidebar_outline = false;
        self.sidebar_search.set(false, SIDEBAR_SLIDE, SIDEBAR_SLIDE);
        settings::update(cx, |s| s.sidebar_visible = true);
        self.leave_hidden_focus(window, cx);
        self.find_tests(cx);
        cx.notify();
    }

    /// Looks through the project for tests, off the main thread.
    fn find_tests(&mut self, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        self.tests_task = Some(cx.spawn(async move |this, cx| {
            let found = cx.background_executor().spawn(async move { crate::test_at::discover(&root) }).await;
            this.update(cx, |this, cx| {
                this.tests = Some(std::rc::Rc::new(found));
                cx.notify();
            })
            .ok();
        }));
    }

    /// A file saved while the tests show: its own tests read again.
    fn tests_saved(&mut self, path: PathBuf, text: String, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        if self.tests.is_none() || !path.starts_with(&root) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move { crate::test_at::tests_in_file(&root, &path, &text) }
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(tests) = &mut this.tests else { return };
                let tests = std::rc::Rc::make_mut(tests);
                tests.retain(|f| f.path != path);
                if let Some(found) = found {
                    let at = tests.partition_point(|f| f.path < found.path);
                    tests.insert(at, found);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Runs `command` in the terminal for `which` tests: they show as running until what it
    /// prints says how they did.
    fn run_tests(&mut self, command: String, which: Vec<(PathBuf, String)>, window: &mut Window, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            if tab.editor.read(cx).buffer.is_dirty() && tab.editor.read(cx).path().is_some() {
                tab.editor.update(cx, |e, cx| e.save_to_disk(cx));
            }
        }
        // pytest says which passed only when asked to (-v).
        let command = command
            .split("; ")
            .map(|part| match part.split_once(" -m pytest") {
                Some((python, rest)) if !rest.starts_with(" -v") => format!("{python} -m pytest -v{rest}"),
                _ => part.to_string(),
            })
            .collect::<Vec<_>>()
            .join("; ");
        // One run at a time is followed: what an earlier one ran and didn't report is unknown.
        if let Some((_, earlier)) = self.tests_running.take() {
            for test in earlier {
                if self.test_status.get(&test) == Some(&TestStatus::Running) {
                    self.test_status.remove(&test);
                }
            }
        }
        for test in &which {
            self.test_status.insert(test.clone(), TestStatus::Running);
        }
        self.tests_running = Some((command.clone(), which));
        self.run_in_terminal(command, None, window, cx);
        cx.notify();
    }

    /// An environment may have been made or removed: looked for again.
    fn python_env_changed(&mut self, cx: &mut Context<Self>) {
        self.python_env_here = None;
        let root = self.tree.read(cx).root().to_path_buf();
        let now = crate::python_env::find(&root);
        // Another environment: Python's language server reads imports from it.
        if now != self.python_env {
            self.python_env = now;
            self.lsp.update(cx, |lsp, cx| lsp.restart_python(cx));
        }
    }

    /// How the tests did, from what a run printed (one run from the list, or typed).
    fn read_test_results(&mut self, ran: &crate::terminal::Ran) {
        let results = crate::test_at::results(&ran.output);
        // The terminal reports the command from its first program on (`(cd web && npx jest x)`
        // comes back as `npx jest x`): ours if it ends ours.
        let ours = self.tests_running.as_ref().is_some_and(|(command, _)| {
            *command == ran.command || (ran.command.len() >= 8 && command.ends_with(ran.command.as_str()))
        });
        let running = if ours { self.tests_running.take().map(|(_, which)| which) } else { None };
        if results.is_empty() && running.is_none() {
            return;
        }
        let known: Vec<(PathBuf, String)> = self
            .tests
            .iter()
            .flat_map(|files| files.iter())
            .flat_map(|f| f.tests.iter().map(|t| (f.path.clone(), t.id.clone())))
            .collect();
        // A test reported more than once (each case of a parametrized one): failed if any did.
        let mut outcomes: Vec<(String, bool)> = Vec::new();
        for (id, passed) in results {
            match outcomes.iter_mut().find(|(known, _)| *known == id) {
                Some((_, ok)) => *ok &= passed,
                None => outcomes.push((id, passed)),
            }
        }
        for (id, passed) in outcomes {
            let status = if passed { TestStatus::Passed } else { TestStatus::Failed };
            // The ones run, when it was from the list; any of that name, when typed.
            let targets: Vec<&(PathBuf, String)> = match &running {
                Some(which) => which.iter().filter(|(_, n)| *n == id).collect(),
                None => known.iter().filter(|(_, n)| *n == id).collect(),
            };
            for target in targets {
                self.test_status.insert(target.clone(), status);
            }
        }
        // Run, and not said how they did (it didn't build, say): not known.
        for test in running.into_iter().flatten() {
            if self.test_status.get(&test) == Some(&TestStatus::Running) {
                self.test_status.remove(&test);
            }
        }
    }

    fn render_tests(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let quiet = |text: &str| {
            div().px(px(18.)).pt(px(14.)).text_size(px(ui::T_MD)).text_color(theme.faint).child(text.to_string()).into_any_element()
        };
        let Some(files) = self.tests.clone() else { return quiet("Looking for tests…") };
        if files.is_empty() {
            return quiet("No tests in this project (Rust, Go, Python, JavaScript, TypeScript)");
        }
        // Above the list: how many, how many failed, and running them all (or the failed).
        let all: Vec<(PathBuf, String)> =
            files.iter().flat_map(|f| f.tests.iter().map(|t| (f.path.clone(), t.id.clone()))).collect();
        let failed: Vec<(PathBuf, String)> =
            all.iter().filter(|k| self.test_status.get(*k) == Some(&TestStatus::Failed)).cloned().collect();
        let failed_set: std::collections::HashSet<&(PathBuf, String)> = failed.iter().collect();
        let everything = crate::test_at::run_everything(&files);
        let failed_command: Vec<String> = files
            .iter()
            .flat_map(|f| {
                let failed_set = &failed_set;
                f.tests
                    .iter()
                    .filter(move |t| failed_set.contains(&(f.path.clone(), t.id.clone())))
                    .map(|t| t.command.clone())
            })
            .collect();
        let count = if all.len() == 1 { "1 test".to_string() } else { format!("{} tests", all.len()) };
        let summary = if failed.is_empty() { count } else { format!("{count} · {} failed", failed.len()) };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px(px(6.))
                .py(px(2.))
                .rounded(px(ui::R_KEY))
                .cursor_pointer()
                .text_color(theme.muted)
                .hover(|s| s.text_color(theme.foreground).bg(theme.hairline))
                .active(|s| s.opacity(0.7))
                .child(label)
        };
        let header = div()
            .flex_none()
            .h(px(ui::ROW))
            .mx(px(6.))
            .pl(px(12.))
            .pr(px(4.))
            .mt(px(6.))
            .flex()
            .items_center()
            .gap(px(2.))
            .text_size(px(ui::T_SM))
            .child(div().flex_1().min_w_0().truncate().text_color(theme.faint).child(summary))
            .when(!failed.is_empty(), |d| {
                let (command, which) = (failed_command.join("; "), failed.clone());
                d.child(button("tests-run-failed", "Run failed").on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| this.run_tests(command.clone(), which.clone(), window, cx),
                )))
            })
            .when_some(everything, |d, command| {
                let which = all.clone();
                d.child(button("tests-run-all", "Run all").on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| this.run_tests(command.clone(), which.clone(), window, cx),
                )))
            });
        // A row for each file, then one for each of its tests.
        let rows: Vec<(usize, Option<usize>)> = files
            .iter()
            .enumerate()
            .flat_map(|(f, file)| std::iter::once((f, None)).chain((0..file.tests.len()).map(move |t| (f, Some(t)))))
            .collect();
        let root = self.tree.read(cx).root().to_path_buf();
        let status = self.test_status.clone();
        let list = uniform_list(
            "tests",
            rows.len(),
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                let theme = cx.global::<Theme>().clone();
                range
                    .map(|ix| {
                        let (f, t) = rows[ix];
                        let file = &files[f];
                        let path = file.path.clone();
                        let group: SharedString = format!("test-row-{ix}").into();
                        let run = |command: Option<String>, which: Vec<(PathBuf, String)>| {
                            div()
                                .id(("test-run", ix))
                                .flex_none()
                                .size(px(20.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(ui::R_KEY))
                                .cursor_pointer()
                                .invisible()
                                .group_hover(group.clone(), |s| s.visible())
                                .hover(|s| s.bg(theme.hairline))
                                .child(svg().path("icons/play.svg").size(px(10.)).text_color(theme.muted))
                                .when_some(command, |d, command| {
                                    d.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.run_tests(command.clone(), which.clone(), window, cx)
                                    }))
                                })
                        };
                        let line = match t {
                            None => {
                                let shown = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().into_owned();
                                let which: Vec<(PathBuf, String)> =
                                    file.tests.iter().map(|t| (path.clone(), t.id.clone())).collect();
                                div()
                                    .id(("test-file", ix))
                                    .group(group.clone())
                                    .h(px(ui::ROW))
                                    .pl(px(12.))
                                    .pr(px(6.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .rounded(px(ui::R_ROW))
                                    .cursor_pointer()
                                    .text_size(px(ui::T_SM))
                                    .text_color(theme.faint)
                                    .hover(|s| s.bg(theme.hairline.opacity(0.6)))
                                    .child(div().flex_1().min_w_0().truncate().child(shown))
                                    .child(run(file.command.clone(), which))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        this.open_file(path.clone(), window, cx)
                                    }))
                            }
                            Some(t) => {
                                let test = &file.tests[t];
                                let key = (path.clone(), test.id.clone());
                                let dot = match status.get(&key) {
                                    Some(TestStatus::Passed) => theme.git_added,
                                    Some(TestStatus::Failed) => theme.error,
                                    Some(TestStatus::Running) => theme.caret,
                                    None => theme.line_strong,
                                };
                                let at = lsp_types::Position::new(test.line as u32, 0);
                                div()
                                    .id(("test", ix))
                                    .group(group.clone())
                                    .h(px(ui::ROW))
                                    .pl(px(22.))
                                    .pr(px(6.))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .rounded(px(ui::R_ROW))
                                    .cursor_pointer()
                                    .text_size(px(ui::T_MD))
                                    .text_color(theme.muted)
                                    .hover(|s| s.bg(theme.hairline.opacity(0.6)).text_color(theme.foreground))
                                    .child(div().flex_none().size(px(7.)).rounded(px(4.)).bg(dot))
                                    // Its own name first; the class it's in (pytest's TestCart::), faint after.
                                    .child({
                                        let (class, own) = test.name.rsplit_once("::").unwrap_or(("", &test.name));
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .items_baseline()
                                            .gap(px(6.))
                                            .child(div().flex_none().max_w_full().truncate().code_font(cx).child(own.to_string()))
                                            .when(!class.is_empty(), |d| {
                                                d.child(
                                                    div()
                                                        .min_w_0()
                                                        .truncate()
                                                        .text_size(px(ui::T_SM))
                                                        .text_color(theme.faint)
                                                        .child(class.replace("::", " › ")),
                                                )
                                            })
                                    })
                                    .child(run(Some(test.command.clone()), vec![key]))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        this.go_to(path.clone(), lsp_types::Range::new(at, at), window, cx)
                                    }))
                            }
                        };
                        div().w_full().px(px(6.)).child(line).into_any_element()
                    })
                    .collect()
            }),
        )
        .track_scroll(self.tests_scroll.clone())
        .flex_1()
        .min_h_0()
        .pt(px(2.));
        div().size_full().flex().flex_col().child(header).child(list).into_any_element()
    }

    /// Goes to line `row` of the open file from its outline, the text taking the keys.
    fn go_to_outline_row(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        let here = self.here(cx);
        self.remember_place(here);
        editor.update(cx, |e, cx| e.go_to_line(row + 1, cx));
        window.focus(&editor.focus_handle(cx));
    }

    /// The outline of `editor`'s file (see `crate::outline`), read again only when its text
    /// changes; None for one longer than `longest` characters.
    fn outline_items(
        &mut self,
        editor: &Entity<Editor>,
        longest: usize,
        cx: &mut Context<Self>,
    ) -> Option<std::rc::Rc<Vec<crate::outline::Item>>> {
        const AGAIN_AFTER: Duration = Duration::from_millis(300);
        let e = editor.read(cx);
        let (id, revision) = (editor.entity_id(), e.buffer.revision());
        if let Some((i, r, items)) = &self.outline
            && *i == id
        {
            if *r == revision {
                return Some(items.clone());
            }
            // Typing in a big file: the outline of a moment ago, worked out again at most
            // every so often (and once typing pauses).
            if e.buffer.len_chars() > 50_000 && self.outline_at.elapsed() < AGAIN_AFTER {
                if self.outline_later.is_none() {
                    self.outline_later = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(AGAIN_AFTER).await;
                        this.update(cx, |this, cx| {
                            this.outline_later = None;
                            cx.notify();
                        })
                        .ok();
                    }));
                }
                return Some(items.clone());
            }
        }
        let e = editor.read(cx);
        if e.buffer.len_chars() > longest {
            return None;
        }
        let path = e.path().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(e.file_name()));
        let items = std::rc::Rc::new(crate::outline::outline(&path, &e.buffer.to_string()));
        self.outline = Some((id, revision, items.clone()));
        self.outline_at = Instant::now();
        Some(items)
    }

    /// The Outline view: the open file's functions and types (its headings, in Markdown),
    /// the one the caret is in marked; a click goes there.
    fn render_outline(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let quiet = |text: String| {
            div().px(px(18.)).pt(px(14.)).text_size(px(ui::T_MD)).text_color(theme.faint).child(text).into_any_element()
        };
        let Some(editor) = self.active_editor().cloned() else {
            return quiet("Open a file to see its outline".into());
        };
        let (id, row, name) = {
            let e = editor.read(cx);
            (editor.entity_id(), e.caret_point().0, e.file_name())
        };
        let Some(items) = self.outline_items(&editor, 1_000_000, cx) else {
            return quiet(format!("{name} is too long for an outline"));
        };
        if items.is_empty() {
            return quiet(format!("No functions or types in {name}"));
        }
        let current = crate::outline::current(&items, row);
        // The caret went into another: the list brings it into view.
        if let Some(ix) = current
            && self.outline_followed != Some((id, ix))
        {
            self.outline_followed = Some((id, ix));
            self.outline_scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        uniform_list(
            "outline",
            items.len(),
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                let theme = cx.global::<Theme>().clone();
                range
                    .map(|ix| {
                        let item = &items[ix];
                        let here = current == Some(ix);
                        let heading = item.kind.starts_with('#');
                        let row = item.row;
                        // The sidebar's width, inset a little so the highlight has rounded ends.
                        let line = div()
                            .id(("outline-row", ix))
                            .w_full()
                            .h(px(ui::ROW))
                            .pl(px(12. + 14. * item.depth.min(8) as f32))
                            .pr(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded(px(ui::R_ROW))
                            .cursor_pointer()
                            .text_size(px(ui::T_MD))
                            .text_color(if here { theme.foreground } else { theme.muted })
                            .when(here, |r| r.bg(theme.hairline))
                            .when(!here, |r| r.hover(|s| s.bg(theme.hairline.opacity(0.6))))
                            // Code by its name in the code's font, the word defining it beside it;
                            // a heading as written, its depth saying its level.
                            .child(
                                div().min_w_0().truncate().when(!heading, |d| d.code_font(cx)).child(item.name.clone()),
                            )
                            .when(!heading, |r| {
                                r.child(
                                    div().flex_none().text_size(px(ui::T_SM)).text_color(theme.faint).child(item.kind),
                                )
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.go_to_outline_row(row, window, cx)
                            }));
                        div().w_full().px(px(6.)).child(line).into_any_element()
                    })
                    .collect()
            }),
        )
        .track_scroll(self.outline_scroll.clone())
        .size_full()
        .pt(px(6.))
        .into_any_element()
    }

    /// Shows the theme of your own called `name`, saying what in it couldn't be read.
    fn use_own_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        match crate::theme::own::load(name) {
            Err(why) => self.show_notice(format!("Couldn't use {name}: {why}"), cx),
            Ok((_, problems)) => {
                let name = name.to_string();
                // Already in use: its file read again (changed elsewhere, maybe).
                if cx.global::<Settings>().own_theme.as_deref() == Some(name.as_str()) {
                    settings::reapply_theme(cx);
                }
                settings::update(cx, |s| s.own_theme = Some(name.clone()));
                if !problems.is_empty() {
                    self.show_notice(format!("{name}: {}", problems.join("; ")), cx);
                }
            }
        }
    }

    /// A theme file of yours was saved: in use, its colours show now.
    fn theme_saved(&mut self, name: &str, cx: &mut Context<Self>) {
        if cx.global::<Settings>().own_theme.as_deref() != Some(name) {
            return;
        }
        match crate::theme::own::load(name) {
            Err(why) => self.show_notice(format!("The theme can't be read: {why}"), cx),
            Ok((_, problems)) => {
                settings::reapply_theme(cx);
                if !problems.is_empty() {
                    self.show_notice(problems.join("; "), cx);
                }
            }
        }
    }

    /// A new theme of your own: the one shown now, every colour written out to change,
    /// opened, and in use (saving it shows the change).
    fn new_own_theme(&mut self, _: &NewOwnTheme, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = crate::theme::own::folder() else { return };
        if let Err(e) = std::fs::create_dir_all(&folder) {
            return self.show_notice(format!("Couldn't make the themes folder: {e}"), cx);
        }
        let name = (1..)
            .map(|n| if n == 1 { "My Theme".to_string() } else { format!("My Theme {n}") })
            .find(|n| !folder.join(format!("{n}.json")).exists())
            .unwrap_or_default();
        let path = folder.join(format!("{name}.json"));
        // The colours on screen (one of yours, maybe), made from Null's theme under them.
        let base = cx.global::<Settings>().shown_theme(cx);
        let starter = crate::theme::own::starter(base, cx.global::<Theme>());
        if let Err(e) = std::fs::write(&path, starter) {
            return self.show_notice(format!("Couldn't write the theme: {e}"), cx);
        }
        settings::update(cx, |s| s.own_theme = Some(name.clone()));
        self.open_file(path, window, cx);
        let message = if cx.global::<Settings>().shown_own_theme(cx).is_none() {
            // Following the Mac, light now: yours is the dark one.
            format!("{name}: shown when the Mac is dark (Null follows its light and dark)")
        } else {
            format!("{name}: change a colour and save to see it")
        };
        self.show_notice(message, cx);
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
        let outlining = !searching && self.sidebar_outline;
        let testing = !searching && self.sidebar_tests;
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
                    .child(tab("files", "Files", !searching && !outlining && !testing).active(|s| s.opacity(0.7)).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.show_files(&ShowFiles, window, cx)),
                    ))
                    .child(tab("outline", "Outline", outlining).active(|s| s.opacity(0.7)).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.show_outline(&ShowOutline, window, cx)),
                    ))
                    .child(tab("tests", "Tests", testing).active(|s| s.opacity(0.7)).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.show_tests(&ShowTests, window, cx)),
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
        let language = self.active_language(cx);
        settings::update(cx, |s| s.flip(settings::PerLanguage::WordWrap, language));
    }

    /// The language of the file being worked on ("Plain Text" with none).
    fn active_language(&self, cx: &App) -> &'static str {
        self.active_editor().map_or("Plain Text", |e| e.read(cx).language_name())
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
    /// What a command in the terminal printed: its errors and warnings in the project's
    /// files join the Problems list and F8, and a line says how many. They stay until the
    /// same command runs again, or another reports problems of its own.
    fn read_reported(&mut self, ran: &crate::terminal::Ran, cx: &mut Context<Self>) {
        use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
        // What searches and shows files prints `file:line: text` too: not problems.
        const NOT_BUILDS: [&str; 16] = [
            "grep", "egrep", "rg", "ag", "ack", "git", "cat", "bat", "less", "more", "head", "tail", "sed", "awk",
            "find", "fd",
        ];
        let program = |name: &str| name.rsplit('/').next().unwrap_or(name).to_string();
        let first_word = ran.command.split_whitespace().next().map(program).unwrap_or_default();
        if NOT_BUILDS.contains(&program(&ran.name).as_str()) || NOT_BUILDS.contains(&first_word.as_str()) {
            return;
        }
        let name = ran.name.as_str();
        let root = self.tree.read(cx).root().to_path_buf();
        let real_root = root.canonicalize().unwrap_or(root.clone());
        // Where it is, as one path (`build/../src/a.c` is `src/a.c`) written from the project's
        // folder as tabs have it, if it's a project file.
        let resolve = |file: &str| {
            let path = Path::new(file);
            let real = [path.to_path_buf(), ran.folder.join(path), root.join(path)]
                .into_iter()
                .filter(|p| p.is_absolute() && p.is_file())
                .find_map(|p| p.canonicalize().ok())?;
            Some(root.join(real.strip_prefix(&real_root).ok()?))
        };
        let reported: Vec<(PathBuf, Diagnostic)> = crate::problem_matcher::problems(&ran.output)
            .into_iter()
            .filter_map(|p| {
                let path = resolve(&p.file)?;
                let at = Position::new(p.line.saturating_sub(1), p.column.saturating_sub(1));
                let severity = if p.error { DiagnosticSeverity::ERROR } else { DiagnosticSeverity::WARNING };
                let diagnostic = Diagnostic {
                    range: Range::new(at, at),
                    severity: Some(severity),
                    source: Some(name.to_string()),
                    message: p.message,
                    ..Default::default()
                };
                Some((path, diagnostic))
            })
            .collect();
        // Something else run since (`git status`, an editor) leaves the build's problems be.
        if reported.is_empty() && ran.command != self.reported_by {
            return;
        }
        let had = !self.reported.is_empty();
        self.reported = reported;
        self.reported_by = ran.command.clone();
        if self.reported.is_empty() {
            if had {
                cx.notify();
            }
            return;
        }
        let errors = self.reported.iter().filter(|(_, d)| d.severity == Some(DiagnosticSeverity::ERROR)).count();
        let warnings = self.reported.len() - errors;
        let plural = |n: usize, word: &str| if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") };
        let counted = match (errors, warnings) {
            (e, 0) => plural(e, "error"),
            (0, w) => plural(w, "warning"),
            (e, w) => format!("{}, {}", plural(e, "error"), plural(w, "warning")),
        };
        let keys = crate::palette::shortcut(&ShowProblems, cx).unwrap_or_default();
        self.show_notice(format!("{name}: {counted} · {keys} lists them"), cx);
    }

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
        // What the last command in the terminal reported, unless a server says it too.
        for (path, reported) in &self.reported {
            let known = found.iter().any(|(p, d)| {
                p == path
                    && d.range.start.line == reported.range.start.line
                    && d.message.eq_ignore_ascii_case(&reported.message)
            });
            if !known {
                found.push((path.clone(), reported.clone()));
            }
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
                    // (Its messages say where each program went: a target folder set
                    // elsewhere, a workspace's.)
                    let built = std::process::Command::new(cargo)
                        .args(["build", "--message-format=json-render-diagnostics"])
                        .current_dir(&cwd)
                        .output();
                    // Rust's own formatters, so its strings and collections show their contents.
                    (built, crate::debugger::rust_formatters())
                })
                .await;
            let (built, formatters) = built;
            // Where cargo says it put the program, else where it usually goes.
            let program = built
                .as_ref()
                .ok()
                .and_then(|output| built_program(&String::from_utf8_lossy(&output.stdout), program.as_deref()))
                .or(program);
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
                let watches: Vec<AnyElement> = debugger
                    .watches
                    .iter()
                    .enumerate()
                    .map(|(ix, watch)| {
                        let (value, failed) = match &watch.value {
                            Some(Ok(value)) => (value.clone(), false),
                            Some(Err(why)) => (why.clone(), true),
                            None => ("…".to_string(), true),
                        };
                        let group: SharedString = format!("watch-{ix}").into();
                        div()
                            .id(("dbg-watch", ix))
                            .group(group.clone())
                            .flex()
                            .gap(px(10.))
                            .whitespace_nowrap()
                            .child(div().text_color(theme.caret).child(watch.expression.clone()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(if failed { theme.faint } else { theme.muted })
                                    .child(value),
                            )
                            .child(
                                div()
                                    .id(("dbg-unwatch", ix))
                                    .flex_none()
                                    .cursor_pointer()
                                    .invisible()
                                    .group_hover(group, |s| s.visible())
                                    .text_color(theme.faint)
                                    .hover(|s| s.text_color(theme.foreground))
                                    .child("×")
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.debugger.update(cx, |d, cx| d.remove_watch(ix, cx));
                                    })),
                            )
                            .into_any_element()
                    })
                    .collect();
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
                    .child(
                        column("dbg-variables")
                            .children(variables)
                            .when(self.debug_locals.is_empty(), |d| {
                                d.child(div().text_color(theme.faint).child("No local variables here"))
                            })
                            // Expressions watched: their values here, at every stop.
                            .children(watches)
                            .child(
                                div()
                                    .key_context("DebugWatch")
                                    .mt(px(4.))
                                    .text_color(theme.foreground)
                                    .child(self.debug_watch.clone()),
                            ),
                    )
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
        let file = self.active_editor().and_then(|e| e.read(cx).path().map(Path::to_path_buf));
        let mut tasks = crate::tasks::find(&root, file.as_deref(), &cx.global::<Settings>().tasks);
        // Run lately: first, whether the project lists them or they were typed.
        for (command, name) in self.recent_runs.iter().rev() {
            let same_task = name.as_ref().and_then(|name| tasks.iter().position(|t| &t.label == name));
            let task = match same_task.or_else(|| tasks.iter().position(|t| &t.command == command)) {
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
        self.run_in_terminal(run.command, None, window, cx);
    }

    /// Types `command` into the terminal (opening it first if needed) and runs it.
    fn run_in_terminal(&mut self, command: String, task: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.recent_runs.retain(|(c, t)| c != &command && (task.is_none() || t != &task));
        self.recent_runs.insert(0, (command.clone(), task));
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

    /// The Markdown file as a page next to it (`notes.md` → `notes.html`), opened in the
    /// browser to read, print or send. A page of that name Null didn't make isn't written over.
    fn export_html(&mut self, _: &ExportHtml, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        let (path, source) = {
            let e = editor.read(cx);
            match e.path() {
                Some(path) if e.is_markdown() => (path.to_path_buf(), e.buffer.to_string()),
                _ => return self.show_notice("Export as HTML is for a saved Markdown file.".into(), cx),
            }
        };
        let out = path.with_extension("html");
        let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // A page that can't be read, or isn't text, isn't taken for one of Null's.
        let mark = crate::markdown_html::MARK.as_bytes();
        let ours = std::fs::read(&out).is_ok_and(|bytes| bytes.windows(mark.len()).any(|w| w == mark));
        if out.exists() && !ours {
            return self
                .show_notice(format!("{name} is already there, and not one Null made: it's left as it is."), cx);
        }
        let title = crate::markdown_html::title(&source)
            .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        let html = crate::markdown_html::page(&source, &title);
        match crate::fs_ops::write_file(&out, html.as_bytes()) {
            Ok(()) => {
                self.show_notice(format!("Exported {name}."), cx);
                crate::file_tree::open_in_browser(&out, cx);
            }
            Err(error) => self.show_notice(format!("Couldn't export it: {error}"), cx),
        }
    }

    /// The last things copied or cut in Null, newest first: ↵ pastes one (it's on the
    /// clipboard again too).
    fn paste_from_history(&mut self, _: &PasteFromHistory, window: &mut Window, cx: &mut Context<Self>) {
        let entries = crate::clipboard_history::entries(cx);
        if entries.is_empty() {
            return self.show_notice("Nothing copied yet.".into(), cx);
        }
        let now = std::time::Instant::now();
        let locations = entries
            .iter()
            .enumerate()
            .map(|(i, (item, when))| {
                let (first, lines) = crate::clipboard_history::preview(&item.text().unwrap_or_default());
                let minutes = now.duration_since(*when).as_secs() / 60;
                let ago = match minutes {
                    0 => "just now".to_string(),
                    1 => "1 minute ago".to_string(),
                    m if m < 60 => format!("{m} minutes ago"),
                    m => format!("{} h ago", m / 60),
                };
                let lines = if lines == 1 { String::new() } else { format!("{lines} lines · ") };
                crate::palette::Location {
                    path: PathBuf::new(),
                    position: lsp_types::Position { line: i as u32, character: 0 },
                    text: format!("{first}\t{lines}{ago}"),
                    kind: crate::palette::LocationKind::Commit,
                }
            })
            .collect();
        self.open_locations("Paste from History".into(), locations, window, cx);
        self.clipboard_list = Some(entries.into_iter().map(|(item, _)| item).collect());
    }

    /// The selected lines (or the caret's line) run in the terminal, opened if it isn't;
    /// the keyboard stays in the code. With no selection the caret goes on to the next line,
    /// to run lines one by one.
    fn run_selection_in_terminal(&mut self, _: &RunSelectionInTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        let text = editor.update(cx, |e, cx| {
            let selected = e.selected_text();
            if !selected.is_empty() {
                return selected;
            }
            let (line, _) = e.caret_point();
            let text = e.buffer.line_text(line);
            let next = (line + 1).min(e.buffer.len_lines().saturating_sub(1));
            e.set_caret_point((next, 0), cx);
            text
        });
        if text.trim().is_empty() {
            return;
        }
        if self.terminals.is_empty() && !self.add_terminal(window, cx) {
            return;
        }
        self.terminal_open.set(true, TERMINAL_SLIDE, TERMINAL_SLIDE);
        if let Some(terminal) = self.terminal().cloned() {
            terminal.update(cx, |t, cx| t.run_text(&text, cx));
        }
        cx.notify();
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
        // In a Python project with its own environment: made the shell's, as activating does.
        let root = self.tree.read(cx).root().to_path_buf();
        let activate = (dir.starts_with(&root))
            .then(|| crate::python_env::find_for(&root, &dir))
            .flatten()
            .and_then(|env| {
                let shell = std::env::var("SHELL")
                    .ok()
                    .filter(|s| !s.is_empty())
                    .or_else(crate::terminal_watch::account_shell)
                    .unwrap_or_default();
                // From where the shell starts: short when it's the project's folder.
                let env = env.strip_prefix(&dir).map(Path::to_path_buf).unwrap_or(env);
                crate::python_env::activate_command(&env, &shell)
            });
        let shell = match Shell::start(dir) {
            Ok(shell) => shell,
            Err(err) => {
                self.show_notice(format!("Couldn't start a terminal: {err}"), cx);
                return false;
            }
        };
        let terminal = cx.new(|cx| {
            let mut view = TerminalView::new(shell, cx);
            if let Some(command) = &activate {
                view.prepare(command, cx);
            }
            view
        });
        let subscription = cx.subscribe_in(&terminal, window, |this, terminal, event, window, cx| match event {
            TerminalEvent::TitleChanged => cx.notify(),
            // The shell exited (e.g. `exit`): its tab goes; with none left, so does the panel.
            TerminalEvent::Exited => this.remove_terminal(terminal.entity_id(), window, cx),
            TerminalEvent::Finished(name, took) => {
                let message = format!("{name} finished in the terminal ({})", crate::terminal_watch::took_words(*took));
                // On screen: the terminal shown, or either of two side by side.
                let in_pair = this.shown_pair().is_some_and(|(a, b)| &a == terminal || &b == terminal);
                let shown = this.terminal_open.on && (in_pair || this.terminal().is_some_and(|t| t == terminal));
                if !window.is_window_active() {
                    crate::terminal_watch::bounce_dock();
                    this.notice_on_return = Some(message);
                } else if !shown {
                    this.show_notice(message, cx);
                }
            }
            TerminalEvent::Ran(ran) => {
                this.read_test_results(ran);
                this.read_reported(ran, cx);
                // `poetry install`, `poetry env use`: Poetry's environments are kept outside
                // the project, where no change is seen.
                if ran.command.split_whitespace().any(|word| word == "poetry") {
                    this.python_env_changed(cx);
                }
            }
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
        // Being named: the name goes with it.
        if self.terminal_rename.as_ref().is_some_and(|r| r.terminal == id) {
            self.terminal_rename = None;
        }
        // One of a pair: the other goes back to being alone, and is the one shown.
        if let Some((a, b)) = self.terminal_pair.filter(|(a, b)| *a == id || *b == id) {
            self.terminal_pair = None;
            let partner = if a == id { b } else { a };
            if let Some(at) = self.terminals.iter().position(|(t, _)| t.entity_id() == partner) {
                self.active_terminal = if at > ix { at - 1 } else { at };
                drop(self.terminals.remove(ix));
                if let Some(next) = self.terminal().cloned().filter(|_| self.terminal_open.on) {
                    window.focus(&next.focus_handle(cx));
                }
                return cx.notify();
            }
        }
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

    /// ⌘D in the terminal: another one beside it, both shown.
    fn split_terminal(&mut self, _: &SplitTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let Some(left) = self.terminal().map(|t| t.entity_id()) else {
            return self.new_terminal(&NewTerminal, window, cx);
        };
        // Already two side by side: the new one pairs with the current instead.
        if !self.add_terminal(window, cx) {
            return;
        }
        let right = self.terminal().map(|t| t.entity_id()).unwrap_or(left);
        self.terminal_pair = Some((left, right));
        self.terminal_open.set(true, TERMINAL_SLIDE, TERMINAL_SLIDE);
        if let Some(terminal) = self.terminal() {
            window.focus(&terminal.focus_handle(cx));
        }
        cx.notify();
    }

    /// The two terminals shown side by side now, if the current one is of a pair.
    fn shown_pair(&self) -> Option<(Entity<TerminalView>, Entity<TerminalView>)> {
        let (left, right) = self.terminal_pair?;
        let current = self.terminal()?.entity_id();
        if current != left && current != right {
            return None;
        }
        let find = |id| self.terminals.iter().find(|(t, _)| t.entity_id() == id).map(|(t, _)| t.clone());
        Some((find(left)?, find(right)?))
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

    /// While the terminal is hidden: the first terminal running something, and what.
    fn terminal_busy(&self, cx: &App) -> Option<(usize, String)> {
        if self.terminal_open.on {
            return None;
        }
        self.terminals.iter().enumerate().find_map(|(ix, (t, _))| t.read(cx).running.clone().map(|name| (ix, name)))
    }

    /// Names the shown terminal: a field in place of its name.
    fn rename_terminal(&mut self, _: &RenameTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = self.terminal().cloned() else { return };
        if !self.terminal_open.on {
            self.terminal_open.set(true, TERMINAL_SLIDE, TERMINAL_SLIDE);
        }
        let current = terminal.read(cx).name.clone().unwrap_or_default();
        let input = cx.new(|cx| {
            let mut input = crate::text_input::TextInput::new("Name", cx);
            input.set_text(&current, cx);
            input
        });
        window.focus(&input.focus_handle(cx));
        // Clicking away keeps what was typed, as in the Finder.
        let blur = cx.on_blur(&input.focus_handle(cx), window, |this, window, cx| {
            this.confirm_terminal_name(&ConfirmTerminalName, window, cx)
        });
        self.terminal_rename = Some(TerminalRename { terminal: terminal.entity_id(), input, _blur: blur });
        cx.notify();
    }

    /// The name typed: the terminal's from now on (none: its folder's again).
    fn confirm_terminal_name(&mut self, _: &ConfirmTerminalName, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.terminal_rename.take() else { return };
        let name = rename.input.read(cx).text().trim().to_string();
        // The terminal it was opened for, wherever it is now.
        let named = self.terminals.iter().find(|(t, _)| t.entity_id() == rename.terminal).map(|(t, _)| t.clone());
        if let Some(terminal) = named {
            terminal.update(cx, |t, cx| {
                t.name = (!name.is_empty()).then_some(name);
                cx.notify();
            });
            if rename.input.focus_handle(cx).is_focused(window) {
                window.focus(&terminal.focus_handle(cx));
            }
        }
        cx.notify();
    }

    fn cancel_terminal_name(&mut self, _: &CancelTerminalName, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal_rename = None;
        if let Some(terminal) = self.terminal() {
            window.focus(&terminal.focus_handle(cx));
        }
        cx.notify();
    }

    /// What a terminal's tab says: the name given to it, else its folder's.
    fn terminal_tab_label(terminal: &TerminalView, ix: usize) -> String {
        Self::named_terminal_label(terminal.name.as_deref(), &terminal.title, ix)
    }

    /// Labels told apart: a second "qa" is "qa 2" (two terminals in one folder).
    fn distinct_labels(labels: Vec<String>) -> Vec<String> {
        let mut out: Vec<String> = Vec::with_capacity(labels.len());
        for label in &labels {
            // The next number no tab has, shown or named so ("qa 2" named by you).
            let (mut shown, mut n) = (label.clone(), 1);
            while out.contains(&shown) || (n > 1 && labels.contains(&shown)) {
                n += 1;
                shown = format!("{label} {n}");
            }
            out.push(shown);
        }
        out
    }

    fn named_terminal_label(name: Option<&str>, title: &str, ix: usize) -> String {
        name.map_or_else(|| Self::terminal_label(title, ix), str::to_string)
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
        // Where a terminal's new name is typed, in place of its name.
        let name_field = |input: Entity<crate::text_input::TextInput>| {
            div()
                .key_context("TerminalName")
                .w(px(140.))
                .px(px(4.))
                .rounded(px(ui::R_KEY))
                .border_1()
                .border_color(theme.caret)
                .text_color(theme.foreground)
                .child(input)
        };
        // One terminal: its name and title. More: a tab each.
        let several = self.terminals.len() > 1;
        let labels = Self::distinct_labels(
            self.terminals.iter().enumerate().map(|(ix, (t, _))| Self::terminal_tab_label(t.read(cx), ix)).collect(),
        );
        let tabs = self.terminals.iter().enumerate().map(|(ix, (t, _))| {
            let active = ix == self.active_terminal;
            let label = labels[ix].clone();
            let renaming =
                self.terminal_rename.as_ref().filter(|r| r.terminal == t.entity_id()).map(|r| r.input.clone());
            // Another terminal busy with something: a dot says so.
            let busy = !active && t.read(cx).running.is_some();
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
                .gap(px(5.))
                .map(|d| match renaming {
                    Some(input) => d.child(name_field(input)),
                    None => d.child(label),
                })
                .when(busy, |d| d.child(div().size(px(5.)).rounded_full().bg(theme.caret.opacity(0.7))))
                .active(|s| s.opacity(0.7))
                // A double-click names it.
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.show_terminal(ix, window, cx);
                    if event.click_count() == 2 {
                        this.rename_terminal(&RenameTerminal, window, cx);
                    }
                }))
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
                    let named = terminal.read(cx).name.clone();
                    bar.child(div().text_color(theme.foreground).child("Terminal")).child(
                        match self.terminal_rename.as_ref().map(|r| r.input.clone()) {
                            Some(input) => div().flex_1().child(name_field(input)).into_any_element(),
                            None => div()
                                .id("terminal-title")
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.muted)
                                .child(named.unwrap_or(title))
                                .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                                    if event.click_count() == 2 {
                                        this.rename_terminal(&RenameTerminal, window, cx);
                                    }
                                }))
                                .into_any_element(),
                        },
                    )
                }
            })
            .child(
                icon_button("split-terminal", "icons/split.svg")
                    .tooltip(ui::tip("Split the terminal", Some(Box::new(SplitTerminal))))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.split_terminal(&SplitTerminal, window, cx)),
                    ),
            )
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
                .child(div().h(px(TERMINAL_HEIGHT)).flex().flex_col().child(header).child(match self.shown_pair() {
                    // Two side by side, a click in one making it the current.
                    Some((left, right)) => {
                        let pane = |terminal: Entity<TerminalView>| {
                            let id = terminal.entity_id();
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                    if let Some(ix) = this.terminals.iter().position(|(t, _)| t.entity_id() == id) {
                                        this.active_terminal = ix;
                                        cx.notify();
                                    }
                                }))
                                .child(terminal)
                        };
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .child(pane(left))
                            .child(div().w(px(1.)).h_full().flex_none().bg(theme.hairline))
                            .child(pane(right))
                            .into_any_element()
                    }
                    None => div().flex_1().min_h_0().child(terminal.clone()).into_any_element(),
                }))
                .into_any_element(),
        )
    }

    fn toggle_autocomplete(&mut self, _: &ToggleAutocomplete, _: &mut Window, cx: &mut Context<Self>) {
        let language = self.active_language(cx);
        settings::update(cx, |s| s.flip(settings::PerLanguage::Autocomplete, language));
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
        self.apply_edit_after_move(edit, None, cx)
    }

    /// `apply_edit_to_files`, for an edit made for a move (`from`, `to`) that has happened
    /// since: what it says of a file under `from` goes to it where it is now.
    fn apply_edit_after_move(
        &mut self,
        edit: lsp_types::WorkspaceEdit,
        moved: Option<(&Path, &Path)>,
        cx: &mut Context<Self>,
    ) -> (usize, usize, Vec<String>) {
        let mut by_file: Vec<(PathBuf, Vec<lsp_types::TextEdit>)> = Vec::new();
        let mut add = |uri: &lsp_types::Uri, edits: Vec<lsp_types::TextEdit>| {
            let Some(path) = crate::lsp::path_for(uri) else { return };
            let path = match moved {
                Some((from, to)) => moved_path(&path, from, to).unwrap_or(path),
                None => path,
            };
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
                let written = crate::encoding::read_whole(&path).and_then(|(text, encoding)| {
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

    /// The editor in front (for `qa`'s steps).
    #[cfg(debug_assertions)]
    pub(crate) fn qa_editor(&self) -> Option<Entity<Editor>> {
        self.active_editor().cloned()
    }

    /// Runs `command` in the terminal (for `qa`'s steps).
    #[cfg(debug_assertions)]
    pub(crate) fn qa_run(&mut self, command: String, window: &mut Window, cx: &mut Context<Self>) {
        self.run_in_terminal(command, None, window, cx);
    }

    /// A Vim command typed after `:` (see `PaletteKind::Ex`).
    fn run_ex(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let command = text.trim().trim_start_matches(':').trim();
        let Some(editor) = self.active_editor().cloned() else { return };
        window.focus(&editor.focus_handle(cx));
        if command.is_empty() {
            return;
        }
        if let Ok(line) = command.parse::<usize>() {
            return editor.update(cx, |e, cx| e.vim_go_to_line(line, cx));
        }
        match command {
            "w" | "write" => editor.update(cx, |e, cx| e.save_from_keyboard(cx)),
            "wa" | "wall" => self.save_all(&SaveAll, window, cx),
            "q" | "quit" | "close" => self.close_tab(&CloseTab, window, cx),
            // Without saving: the changes go (from disk again), then the tab.
            "q!" | "quit!" => {
                if editor.read(cx).path().is_some() {
                    editor.update(cx, |e, cx| e.revert_to_disk(cx));
                }
                self.close_tab(&CloseTab, window, cx);
            }
            // Formatted first when that's on: the tab closes once it's saved (`SavedToClose`).
            // :x writes only what changed (a file as it was saved isn't formatted again).
            "x" | "xit" if !editor.read(cx).buffer.is_dirty() => {
                self.confirm_unsaved(CloseAction::CloseTabs(vec![editor]), window, cx);
            }
            "wq" | "x" | "xit" => editor.update(cx, |e, cx| e.save_and_close(cx)),
            "qa" | "qa!" | "qall" => self.close_all_tabs(&CloseAllTabs, window, cx),
            "wqa" | "xa" | "wqall" | "xall" => {
                self.save_all(&SaveAll, window, cx);
                self.close_all_tabs(&CloseAllTabs, window, cx);
            }
            "noh" | "nohlsearch" => editor.update(cx, |e, cx| e.close_find(window, cx)),
            _ if command.starts_with('s') || command.starts_with("%s") => {
                match editor.update(cx, |e, cx| e.vim_substitute(command, cx)) {
                    Ok(1) => self.show_notice("1 replaced".into(), cx),
                    Ok(n) => self.show_notice(format!("{n} replaced"), cx),
                    Err(why) => self.show_notice(why, cx),
                }
            }
            _ => self.show_notice(format!("Not a command here: {command}"), cx),
        }
    }

    /// The project's own settings (`.null/settings.json`), made if there are none yet: any
    /// setting there is used over yours in this project.
    fn open_project_settings(&mut self, _: &OpenProjectSettings, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        let path = crate::settings::project_file(&root);
        if !path.exists() {
            let made = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| {
                crate::fs_ops::write_file(
                    &path,
                    b"{\n  // Settings for this project only, over yours: \"indent_size\": 2, \"format_on_save\": true\n  // (Its .vscode/settings.json, if it has one, is read too: these win over it.)\n}\n",
                )
            });
            if let Err(err) = made {
                return self.show_notice(format!("Couldn't make .null/settings.json: {err}"), cx);
            }
        }
        self.open_file(path, window, cx);
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
                    return Some(
                        div().text_color(theme.muted).whitespace_nowrap().child(spaced(tip)).into_any_element(),
                    );
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
                let pinned = tab.pinned;
                let passing = tab.passing && cx.global::<Settings>().preview_tabs;
                let pin_editor = tab.editor.clone();
                // Unsaved: a small dot, which turns into the close button under the pointer.
                // Pinned: a pin in its place, which unpins.
                let close = div()
                    .id(("close", ix))
                    .map(|d| {
                        if pinned {
                            d.tooltip(ui::tip("Unpin", None))
                        } else {
                            d.tooltip(ui::tip("Close", Some(Box::new(CloseTab))))
                        }
                    })
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
                            .when(dirty || (!active && !pinned), |d| d.invisible())
                            .group_hover(group.clone(), |s| s.visible())
                            .child(
                                svg()
                                    .path(if pinned { "icons/pin.svg" } else { "icons/x.svg" })
                                    .size(px(if pinned { 11. } else { 10. }))
                                    .text_color(theme.muted),
                            ),
                    )
                    .active(|s| s.opacity(0.7))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        if pinned {
                            this.set_pinned(&pin_editor, false, cx);
                        } else {
                            this.close_tab_at(ix, window, cx);
                        }
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
                                    // Passing: slanted, until kept.
                                    .when(passing, |d| d.italic())
                                    .child(name),
                            )
                            .children(folder.map(|f| div().flex_none().text_color(theme.faint).child(f))),
                    )
                    .child(close)
                    .active(|s| s.opacity(0.7))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.activate(ix, window, cx);
                        // A double-click keeps a passing tab.
                        if event.click_count() >= 2
                            && let Some(editor) = this.tabs.get(ix).map(|t| t.editor.clone())
                        {
                            this.keep_tab(&editor, cx);
                        }
                    }))
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
        let mut items = vec![if self.tabs[ix].pinned { Unpin } else { Pin }, Close];
        if self.tabs.len() > 1 {
            items.push(CloseOthers);
        }
        let side = self.side_tabs(self.tabs[ix].side);
        if side.last() != Some(&ix) {
            items.push(CloseToTheRight);
        }
        // To a side of its own: right of the others, or below them; or back.
        if self.tabs.len() > 1 {
            if self.tabs[ix].side == 0 {
                items.extend([MoveRight, MoveDown]);
            } else {
                items.push(MoveBack);
            }
        }
        if editor.read(cx).path().is_some() {
            items.extend([CopyPath, CopyRelativePath, Reveal, OpenInTerminal, OtherSide]);
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
            TabMenuItem::Pin | TabMenuItem::Unpin => self.set_pinned(&editor, item == TabMenuItem::Pin, cx),
            TabMenuItem::Close => self.close_tab_at(ix, window, cx),
            TabMenuItem::CloseOthers => {
                let others = self.unpinned().filter(|e| *e != editor).collect();
                self.confirm_unsaved(CloseAction::CloseTabs(others), window, cx);
            }
            TabMenuItem::CloseToTheRight => {
                let side = self.side_tabs(self.tabs[ix].side);
                let after = side.iter().skip_while(|&&i| i != ix).skip(1);
                let editors = after.filter(|&&i| !self.tabs[i].pinned).map(|&i| self.tabs[i].editor.clone()).collect();
                self.confirm_unsaved(CloseAction::CloseTabs(editors), window, cx);
            }
            TabMenuItem::CopyPath => self.copy_path(path.as_deref(), false, cx),
            TabMenuItem::CopyRelativePath => self.copy_path(path.as_deref(), true, cx),
            TabMenuItem::Reveal => {
                if let Some(path) = path {
                    cx.reveal_path(&path);
                }
            }
            // A terminal in the file's folder.
            TabMenuItem::OpenInTerminal => {
                if let Some(dir) = path.as_deref().and_then(Path::parent) {
                    self.open_terminal_in(dir.to_path_buf(), window, cx);
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
            TabMenuItem::MoveRight | TabMenuItem::MoveDown | TabMenuItem::MoveBack => {
                self.activate(ix, window, cx);
                match item {
                    TabMenuItem::MoveRight => self.move_tab_to(1, Some(false), window, cx),
                    TabMenuItem::MoveDown => self.move_tab_to(1, Some(true), window, cx),
                    _ => self.move_tab_to(0, None, window, cx),
                }
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
        crate::system_clipboard::write(cx, gpui::ClipboardItem::new_string(shown.display().to_string()));
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

/// Shows a theme of your own (`themes/<name>.json`).
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct UseOwnTheme {
    pub name: String,
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
            (true, Some(words)) => match reading_time(words) {
                Some(time) => format!("{place} · {} · {time}", plural(words)),
                None => format!("{place} · {}", plural(words)),
            },
            (true, None) => place,
            (false, _) => match editor.selected_words() {
                Some(words) => format!("{place} · {} selected", plural(words)),
                None => place,
            },
        };
    }
    // Data in columns: which one the caret's in, by its name in the first line.
    if let Some((column, name)) = editor.data_column().filter(|_| range.is_empty()) {
        return match name {
            Some(name) => format!("{place} · {name} (column {column})"),
            None => format!("{place} · column {column}"),
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

/// About how long `words` take to read, at 230 a minute; nothing under half a minute.
fn reading_time(words: usize) -> Option<String> {
    let minutes = (words + 115) / 230;
    (minutes > 0).then(|| format!("{minutes} min read"))
}

/// Of the actions a server offers for organizing imports, the one to run: its own
/// `source.organizeImports` (some servers send others too, or a command alone).
fn organize_action(actions: Vec<lsp_types::CodeActionOrCommand>) -> Option<lsp_types::CodeActionOrCommand> {
    use lsp_types::{CodeActionKind, CodeActionOrCommand};
    let organizes = |a: &CodeActionOrCommand| match a {
        CodeActionOrCommand::CodeAction(action) => {
            action.disabled.is_none()
                && action
                    .kind
                    .as_ref()
                    .is_some_and(|k| k.as_str().starts_with(CodeActionKind::SOURCE_ORGANIZE_IMPORTS.as_str()))
        }
        CodeActionOrCommand::Command(_) => false,
    };
    let mut actions = actions;
    // A lone answer counts only when it says nothing else: a command, or an action of no kind.
    let plain = |a: &CodeActionOrCommand| match a {
        CodeActionOrCommand::Command(_) => true,
        CodeActionOrCommand::CodeAction(action) => action.kind.is_none() && action.disabled.is_none(),
    };
    let at = actions.iter().position(organizes).or_else(|| (actions.len() == 1 && plain(&actions[0])).then_some(0))?;
    Some(actions.swap_remove(at))
}

/// A status bar text with room around its `·`: the interface font's spaces are narrow, and
/// "Col 1 · 3 words" read as "Col 1·3 words". En spaces instead.
fn spaced(text: &str) -> String {
    text.replace(" · ", "\u{2002}·\u{2002}")
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
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
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
        // The files open in tabs, for a search of those only.
        let open: Vec<PathBuf> =
            self.tabs.iter().filter_map(|t| t.editor.read(cx).path().map(Path::to_path_buf)).collect();
        if self.project_search.read(cx).open_files != open {
            self.project_search.update(cx, |search, _| search.open_files = open);
        }
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
        } else if self.sidebar_outline && sidebar_width > 0.5 {
            // (Read only while it shows.)
            div().flex_1().min_h_0().child(self.render_outline(cx))
        } else if self.sidebar_tests && sidebar_width > 0.5 {
            div().flex_1().min_h_0().child(self.render_tests(cx))
        } else {
            div().flex_1().min_h_0().child(self.tree.clone())
        };

        let root = self.tree.read(cx).root().to_path_buf();
        let lsp_status = self.language_status(cx);
        let conflicts = self.active_editor().map_or(0, |e| e.read(cx).conflicts().len());
        // Where the caret is in the code, after the file's name: `Shop › checkout`, each
        // with its row (a click goes there).
        let trail: Vec<(String, usize)> = self
            .active_editor()
            .cloned()
            .and_then(|editor| {
                let (row, plain) = {
                    let e = editor.read(cx);
                    (e.caret_point().0, e.preview.is_none() && !e.reading && e.extra.is_empty())
                };
                if !plain {
                    return None;
                }
                let items = self.outline_items(&editor, 300_000, cx)?;
                Some(crate::outline::trail_items(&items, row).into_iter().map(|i| (i.name.clone(), i.row)).collect())
            })
            .unwrap_or_default();
        let (status_items, problems): (Vec<String>, (usize, usize)) = match self.active_editor().map(|e| e.read(cx)) {
            Some(editor) => {
                let (line, col) = editor.buffer.point(editor.shown_caret(cx));
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
                                // A character that can't be seen, at the caret: named.
                                // Vim's keys: the mode first ("Normal · Ln 4, Col 9"); a character
                                // that can't be seen, at the caret: named after.
                                0 => {
                                    let mut label = match editor.vim_mode(cx) {
                                        Some(mode) => match editor.vim_recording() {
                                            Some(name) => format!(
                                                "{} · Recording @{name} · {}",
                                                mode.label(),
                                                position_label(editor, line, col)
                                            ),
                                            None => format!("{} · {}", mode.label(), position_label(editor, line, col)),
                                        },
                                        None => position_label(editor, line, col),
                                    };
                                    if let Some(invisible) = editor.invisible_at_caret() {
                                        label = format!("{label} · {invisible}");
                                    }
                                    label
                                }
                                // Vim's block: its cursors are the block's lines.
                                n if editor.vim_mode(cx) == Some(crate::editor::vim::Mode::VisualBlock) => {
                                    match editor.vim_recording() {
                                        Some(name) => format!("Visual Block · Recording @{name} · {} lines", n + 1),
                                        None => format!("Visual Block · {} lines", n + 1),
                                    }
                                }
                                n => format!("{} cursors · Esc for one", n + 1),
                            },
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
        // Python with the project's own environment: which one ("Python (.venv)").
        let root_now = self.tree.read(cx).root().to_path_buf();
        let editor = self.active_editor().map(|e| e.read(cx)).filter(|e| e.preview.is_none());
        let python = editor.filter(|e| e.language_name() == "Python").and_then(|e| e.path().map(Path::to_path_buf));
        let env = python.map(|path| {
            // (Looked for once per file, not every frame.)
            if self.python_env_here.as_ref().is_none_or(|(of, _)| *of != path) {
                let env = crate::python_env::find_for(&root_now, &path);
                self.python_env_here = Some((path, env));
            }
            self.python_env_here.as_ref().and_then(|(_, env)| env.clone())
        });
        let language = self.active_editor().map(|e| e.read(cx)).filter(|e| e.preview.is_none()).map(|e| {
            let name = e.language_name();
            match (name, env.flatten()) {
                ("Python", Some(env)) => format!("{name} ({})", crate::python_env::label(&env, &root_now)),
                _ => name.to_string(),
            }
        });
        let split = self.is_split();
        let stacked = split && self.stacked;
        let tabs_left = self.render_tabs(0, cx);
        // The second side's tabs: in the title bar beside the first's, or over it when below.
        let tabs_right = (split && !stacked).then(|| self.render_tabs(1, cx));
        let tabs_below = stacked.then(|| self.render_tabs(1, cx));
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
        } else if let Some(tabs_below) = tabs_below {
            // One above the other: the lower side's tabs in a row over it.
            let top = div().h(relative(self.split_ratio)).w_full().flex_none().child(pane(self, 0, cx));
            let tab_row = div()
                .h(px(34.))
                .flex_none()
                .flex()
                .items_center()
                .px(px(8.))
                .bg(theme_now.surface)
                .border_b_1()
                .border_color(theme_now.hairline)
                .child(tabs_below);
            let bottom = div()
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .flex_col()
                .child(tab_row)
                .child(div().flex_1().min_h_0().child(pane(self, 1, cx)));
            let divider = div()
                .id("divider")
                .h(px(7.))
                .my(px(-3.))
                .w_full()
                .flex_none()
                .flex()
                .flex_col()
                .justify_center()
                .cursor_row_resize()
                .group("divider")
                .on_drag(DraggedDivider, |_, _, _, cx| cx.new(|_| gpui::EmptyView))
                .child(
                    div()
                        .h(px(1.))
                        .w_full()
                        .bg(theme_now.hairline)
                        .group_hover("divider", |s| s.bg(theme_now.line_strong)),
                );
            div()
                .size_full()
                .flex()
                .flex_col()
                .on_drag_move::<DraggedDivider>(cx.listener(|this, event: &DragMoveEvent<DraggedDivider>, _, cx| {
                    let y = f32::from(event.event.position.y - event.bounds.top());
                    this.split_ratio = (y / f32::from(event.bounds.size.height)).clamp(0.2, 0.8);
                    this.schedule_session_save(cx);
                    cx.notify();
                }))
                .child(top)
                .child(divider)
                .child(bottom)
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
            // While a tab is dragged, the right half offers to open it there, and the bottom
            // below: each shows itself with the tab over it (files dragged from the tree, or
            // the Finder, pass through unseen).
            let dragging_tab = cx.has_active_drag() && self.tabs.len() > 1;
            div().relative().size_full().child(div().size_full().child(pane(self, 0, cx))).when(dragging_tab, |body| {
                body.child(
                    div()
                        .id("drop-below")
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .h(relative(0.35))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(ui::T_MD))
                        .text_color(gpui::transparent_black())
                        .border_t_1()
                        .border_color(gpui::transparent_black())
                        .drag_over::<DraggedTab>({
                            let (tint, text, line) = (theme_now.accent_soft, theme_now.muted, theme_now.hairline);
                            move |style, _, _, _| style.bg(tint).text_color(text).border_color(line)
                        })
                        .child("Open below")
                        .on_drop(cx.listener(|this, dragged: &DraggedTab, window, cx| {
                            this.stacked = true;
                            this.drop_tab(&dragged.editor, None, 1, window, cx)
                        })),
                )
                .child(
                    div()
                        .id("drop-right")
                        .absolute()
                        .top_0()
                        .right_0()
                        .w(relative(0.5))
                        .h(relative(0.65))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(ui::T_MD))
                        .text_color(gpui::transparent_black())
                        .border_l_1()
                        .border_color(gpui::transparent_black())
                        .drag_over::<DraggedTab>({
                            let (tint, text, line) = (theme_now.accent_soft, theme_now.muted, theme_now.hairline);
                            move |style, _, _, _| style.bg(tint).text_color(text).border_color(line)
                        })
                        .child("Open on the right")
                        .on_drop(cx.listener(|this, dragged: &DraggedTab, window, cx| {
                            this.stacked = false;
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
        let default_indent = cx
            .global::<Settings>()
            .indent_for(self.active_editor().map_or("Plain Text", |e| e.read(cx).language_name()));
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

        let mut items = status_items.into_iter().map(|item| spaced(&item));
        // (Text, with lines to go to: not a picture, not a preview.)
        let can_go_to_line =
            self.active_editor().map(|e| e.read(cx)).is_some_and(|e| e.preview.is_none() && !e.reading);
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
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.push_branch(&PushBranch, window, cx)
                                }))
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
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.pull_branch(&PullBranch, window, cx)
                                }))
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
                // The file (a click shows it in the files), then where the caret is in it (a
                // click on a name goes there).
                .child({
                    let file_path = self.active_path(cx);
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .flex()
                        .items_center()
                        .children(items.next().map(|place| {
                            div()
                                .id("status-place")
                                .debug_selector(|| "status-place".into())
                                // The file first: it keeps its room (it's cut short already).
                                .flex_none()
                                .truncate()
                                .when(file_path.is_some(), |d| {
                                    d.cursor_pointer().hover(|s| s.text_color(theme.foreground))
                                })
                                .child(place)
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    if let Some(path) = &file_path {
                                        this.show_files(&ShowFiles, window, cx);
                                        this.tree.update(cx, |tree, cx| tree.show_path(path, cx));
                                    }
                                }))
                        }))
                        .children(trail.iter().enumerate().map(|(i, (name, row))| {
                            let row = *row;
                            // Names give way when there's no room (the deepest last to).
                            div()
                                .flex_shrink()
                                .min_w_0()
                                .flex()
                                .child(div().flex_none().px(px(5.)).text_color(theme.faint).child("›"))
                                .child(
                                    div()
                                        .id(("status-trail", i))
                                        .min_w_0()
                                        .truncate()
                                        .debug_selector(move || format!("status-trail {i}"))
                                        .cursor_pointer()
                                        .hover(|s| s.text_color(theme.foreground))
                                        .child(name.clone())
                                        .active(|s| s.opacity(0.7))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                            this.go_to_outline_row(row, window, cx)
                                        })),
                                )
                        }))
                })
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
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.show_commands_for("indent with", window, cx)
                        }))
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
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.show_commands_for("encoding", window, cx)
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
                            .active(|s| s.opacity(0.7))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.show_commands_for("line endings", window, cx)
                            })),
                    )
                })
                // The terminal hidden while something runs in it: what, and a click shows it.
                .children(self.terminal_busy(cx).map(|(ix, name)| {
                    div()
                        .id("terminal-busy")
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme.foreground))
                        .child(div().size(px(5.)).rounded_full().bg(theme.caret.opacity(0.7)))
                        .child(format!("{name} running"))
                        .tooltip(ui::tip("Show the terminal", Some(Box::new(ToggleTerminal))))
                        .active(|s| s.opacity(0.7))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.active_terminal = ix;
                            this.toggle_terminal(&ToggleTerminal, window, cx);
                        }))
                }))
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
                            .child(
                                div().size(px(6.)).rounded_full().bg(dot).group_hover("ai-status", move |s| s.bg(lit)),
                            )
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
                            .child(if conflicts == 1 {
                                "1 conflict".to_string()
                            } else {
                                format!("{conflicts} conflicts")
                            })
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
                            .active(|s| s.opacity(0.7))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.show_problems(&ShowProblems, window, cx)
                            }))
                            .when(errors > 0, |d| d.child(div().text_color(theme.error).child(plural(errors, "error"))))
                            .when(warnings > 0, |d| {
                                d.child(div().text_color(theme.warning).child(plural(warnings, "warning")))
                            }),
                    )
                })
                // Where the caret is: a click goes to another line (as ⌃G).
                .children(items.map(|item| {
                    div()
                        .id("status-position")
                        .flex_none()
                        .whitespace_nowrap()
                        .when(can_go_to_line, |d| {
                            d.cursor_pointer()
                                .hover(|s| s.text_color(theme.foreground))
                                .tooltip(ui::tip("Go to line", Some(Box::new(GoToLine))))
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.go_to_line(&GoToLine, window, cx)
                                }))
                        })
                        .child(item)
                }))
                // The file's language: a click puts it in another.
                .children(language.map(|name| {
                    div()
                        .id("status-language")
                        .flex_none()
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme.foreground))
                        .child(name)
                        .active(|s| s.opacity(0.7))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.show_commands_for("language: ", window, cx)
                        }))
                }));

        div()
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(|this, event: &gpui::ModifiersChangedEvent, _, _| {
                if !event.modifiers.control {
                    this.settle_switch();
                }
            }))
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
            .on_action(cx.listener(Self::open_project_settings))
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
            .on_action(cx.listener(|this, _: &CopyAsRichText, _, cx| this.copy_as_rich_text(cx)))
            .on_action(cx.listener(|this, _: &OrganizeImports, _, cx| this.organize_imports(cx)))
            .on_action(cx.listener(|this, _: &RevertToSaved, _, cx| this.revert_to_saved(cx)))
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
            .on_action(cx.listener(Self::show_bookmarks))
            .on_action(cx.listener(Self::edit_snippets))
            .on_action(cx.listener(Self::run_selection_in_terminal))
            .on_action(cx.listener(Self::paste_from_history))
            .on_action(cx.listener(Self::export_html))
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
            .on_action(cx.listener(|this, _: &MoveTabRight, window, cx| this.move_tab_to(1, Some(false), window, cx)))
            .on_action(cx.listener(|this, _: &MoveTabDown, window, cx| this.move_tab_to(1, Some(true), window, cx)))
            .on_action(cx.listener(|this, _: &MoveTabUp, window, cx| this.move_tab_to(0, None, window, cx)))
            .on_action(cx.listener(|this, _: &TogglePinTab, _, cx| {
                if let Some(tab) = this.active.and_then(|a| this.tabs.get(a)) {
                    let (editor, pinned) = (tab.editor.clone(), tab.pinned);
                    this.set_pinned(&editor, !pinned, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MoveTabLeft, window, cx| this.move_tab_to(0, None, window, cx)))
            .on_action(cx.listener(|this, _: &OpenOnOtherSide, window, cx| this.open_on_other_side(window, cx)))
            .on_action(cx.listener(Self::go_to_symbol))
            .on_action(cx.listener(Self::go_to_symbol_in_project))
            .on_action(cx.listener(|this, _: &ToggleFormatOnSave, _, cx| {
                let language = this.active_language(cx);
                settings::update(cx, |s| s.flip(settings::PerLanguage::FormatOnSave, language))
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
            .on_action(
                cx.listener(|_, _: &ToggleCodeLens, _, cx| settings::update(cx, |s| s.code_lens = !s.code_lens)),
            )
            .on_action(cx.listener(|_, _: &ToggleBracketColours, _, cx| {
                settings::update(cx, |s| s.bracket_colours = !s.bracket_colours)
            }))
            .on_action(cx.listener(|_, _: &ToggleProblemsAtLineEnds, _, cx| {
                settings::update(cx, |s| s.problems_at_line_ends = !s.problems_at_line_ends)
            }))
            .on_action(cx.listener(Self::toggle_focus_mode))
            .on_action(cx.listener(Self::run_task))
            .on_action(cx.listener(|this, _: &RunTestAtCursor, window, cx| this.run_test(true, window, cx)))
            .on_action(cx.listener(Self::open_preview_to_the_side))
            .on_action(cx.listener(|this, _: &RunTestsInFile, window, cx| this.run_test(false, window, cx)))
            .on_action(cx.listener(Self::open_recent))
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(Self::start_debugging))
            .on_action(cx.listener(Self::show_server_log))
            .on_action(cx.listener(|this, _: &ToggleStopOnErrors, _, cx| {
                // Where the program fails (a Rust panic, a thrown exception): stopped at.
                let on = !this.debugger.read(cx).stop_on_errors;
                this.debugger.update(cx, |d, cx| d.set_stop_on_errors(on, cx));
                let notice = if on { "Debugging stops where the program fails" } else { "Debugging no longer stops on errors" };
                this.show_notice(notice.into(), cx);
            }))
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
            .on_action(cx.listener(|this, _: &AddWatch, _, cx| {
                let expression = this.debug_watch.read(cx).text().to_string();
                this.debugger.update(cx, |d, cx| d.add_watch(&expression, cx));
                this.debug_watch.update(cx, |input, cx| input.set_text("", cx));
            }))
            .on_action(cx.listener(Self::new_terminal))
            .on_action(cx.listener(Self::next_terminal))
            .on_action(cx.listener(Self::split_terminal))
            .on_action(cx.listener(Self::rename_terminal))
            .on_action(cx.listener(Self::confirm_terminal_name))
            .on_action(cx.listener(Self::cancel_terminal_name))
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
            .on_action(
                cx.listener(|_, _: &UseNullTheme, _, cx| settings::update(cx, |s| s.pick_theme(ThemeName::Null))),
            )
            .on_action(cx.listener(|_, _: &CompactLineSpacing, _, cx| {
                settings::update(cx, |s| s.line_spacing = crate::settings::LineSpacing::Compact)
            }))
            .on_action(cx.listener(|_, _: &NormalLineSpacing, _, cx| {
                settings::update(cx, |s| s.line_spacing = crate::settings::LineSpacing::Normal)
            }))
            .on_action(cx.listener(|_, _: &RelaxedLineSpacing, _, cx| {
                settings::update(cx, |s| s.line_spacing = crate::settings::LineSpacing::Relaxed)
            }))
            .on_action(cx.listener(|_, _: &UseAshTheme, _, cx| settings::update(cx, |s| s.pick_theme(ThemeName::Ash))))
            .on_action(
                cx.listener(|_, _: &UseMidnightTheme, _, cx| {
                    settings::update(cx, |s| s.pick_theme(ThemeName::Midnight))
                }),
            )
            .on_action(
                cx.listener(|_, _: &UseMossTheme, _, cx| settings::update(cx, |s| s.pick_theme(ThemeName::Moss))),
            )
            .on_action(
                cx.listener(|_, _: &UsePaperTheme, _, cx| settings::update(cx, |s| s.pick_theme(ThemeName::Paper))),
            )
            .on_action(
                cx.listener(|_, _: &UseDuneTheme, _, cx| settings::update(cx, |s| s.pick_theme(ThemeName::Dune))),
            )
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::search_project))
            .on_action(cx.listener(Self::show_files))
            .on_action(cx.listener(Self::new_own_theme))
            .on_action(cx.listener(|this, action: &UseOwnTheme, _, cx| this.use_own_theme(&action.name, cx)))
            .on_action(cx.listener(|this, _: &SwitchTab, window, cx| {
                let held = window.modifiers().control;
                this.switch_tab(false, held, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SwitchTabBack, window, cx| {
                let held = window.modifiers().control;
                this.switch_tab(true, held, window, cx)
            }))
            .on_action(cx.listener(Self::show_outline))
            .on_action(cx.listener(Self::show_tests))
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

    /// The program cargo built, where it says it put it (a target folder set elsewhere).
    #[test]
    fn the_built_program_is_where_cargo_says() {
        let artifact = |kind: &str, exe: Option<&str>| {
            serde_json::json!({ "reason": "compiler-artifact", "target": { "kind": [kind] }, "executable": exe })
                .to_string()
        };
        let messages = [
            artifact("lib", None),
            "{\"reason\":\"build-finished\",\"success\":true}".to_string(),
            artifact("bin", Some("/elsewhere/debug/helper")),
            artifact("bin", Some("/elsewhere/debug/app")),
            "not json".to_string(),
        ]
        .join("\n");
        let expected = PathBuf::from("/p/target/debug/app");
        assert_eq!(built_program(&messages, Some(&expected)), Some(PathBuf::from("/elsewhere/debug/app")));
        assert_eq!(built_program(&messages, None), None, "several, none expected: not guessed");
        assert_eq!(built_program(&artifact("bin", Some("/t/debug/one")), None), Some(PathBuf::from("/t/debug/one")));
        assert_eq!(built_program(&artifact("lib", None), None), None, "no program");
    }

    #[test]
    fn a_moved_file_is_found_by_its_text() {
        let dir = crate::tools::test_dir("moves");
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
        let dir = crate::tools::test_dir("replace-enc");
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
        let dir = crate::tools::test_dir("git-lists");
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

    /// Revert to Saved: the saved text back, the file clean; ⌘Z brings the changes back.
    #[gpui::test]
    fn revert_to_saved_takes_the_file_back(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("revert-saved");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "saved\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let file = dir.join("a.txt");
        workspace.update_in(cx, |w, window, cx| w.open_file(file, window, cx));
        cx.run_until_parked();
        let editor = workspace.read_with(cx, |w, _| w.active_editor().cloned().unwrap());
        editor.update(cx, |e, cx| {
            e.restore_unsaved("saved\nchanged\n", cx);
            assert!(e.buffer.is_dirty());
        });
        workspace.update(cx, |w, cx| w.revert_to_saved(cx));
        editor.read_with(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "saved\n");
            assert!(!e.buffer.is_dirty());
        });
        editor.update_in(cx, |e, window, cx| window.focus(&e.focus_handle(cx)));
        cx.simulate_keystrokes("cmd-z");
        editor.read_with(cx, |e, _| assert_eq!(e.buffer.to_string(), "saved\nchanged\n", "⌘Z brings the changes back"));
        // The saved file unreadable now: the changes stay, still unsaved.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("a.txt"), std::fs::Permissions::from_mode(0o000)).unwrap();
        workspace.update(cx, |w, cx| w.revert_to_saved(cx));
        editor.read_with(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "saved\nchanged\n");
            assert!(e.buffer.is_dirty(), "not marked saved");
        });
        std::fs::set_permissions(dir.join("a.txt"), std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Vim's :wq and :x save the file and close its tab; :q! closes it without saving.
    #[gpui::test]
    fn vim_write_and_quit_save_then_close(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("vim-wq");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.join(name), "saved\n").unwrap();
        }
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        for (name, command, kept) in [("a.txt", "wq", "changed\n"), ("b.txt", "q!", "saved\n"), ("c.txt", "x", "saved\n")] {
            let file = dir.join(name);
            workspace.update_in(cx, |w, window, cx| w.open_file(file, window, cx));
            cx.run_until_parked();
            let editor = workspace.read_with(cx, |w, _| w.active_editor().cloned().unwrap());
            // (:x on a file as it was saved: closed, nothing written.)
            if command != "x" {
                editor.update(cx, |e, cx| e.restore_unsaved("changed\n", cx));
            }
            workspace.update_in(cx, |w, window, cx| w.run_ex(command, window, cx));
            cx.run_until_parked();
            assert_eq!(std::fs::read_to_string(dir.join(name)).unwrap(), kept, ":{command}");
            assert!(workspace.read_with(cx, |w, _| w.tabs.is_empty()), ":{command} closes the tab");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Copy as Rich Text: the selection's Markdown on the clipboard (the formatted copy goes
    /// to the Mac's own clipboard, which tests leave alone), and in the history.
    #[gpui::test]
    fn copy_as_rich_text_copies_the_markdown(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("rich-text");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.md"), "# Notes\n\nSome **bold** text.\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let file = dir.join("notes.md");
        workspace.update_in(cx, |w, window, cx| w.open_file(file, window, cx));
        cx.run_until_parked();
        workspace.update(cx, |w, cx| {
            w.active_editor()
                .unwrap()
                .update(cx, |e, _| e.selection = crate::editor::Selection { anchor: 9, head: 27 });
            w.copy_as_rich_text(cx);
        });
        let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|i| i.text()));
        assert_eq!(copied.as_deref(), Some("Some **bold** text"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Edit Snippets… opens the snippets file for the file's language, made with a how-to.
    #[gpui::test]
    fn edit_snippets_opens_the_languages_file(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("edit-snippets");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.go"), "package main\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let file = dir.join("main.go");
        workspace.update_in(cx, |w, window, cx| w.open_file(file, window, cx));
        cx.run_until_parked();
        workspace.update_in(cx, |w, window, cx| w.edit_snippets(&EditSnippets, window, cx));
        cx.run_until_parked();
        let (name, text) = workspace.read_with(cx, |w, cx| {
            let e = w.active_editor().unwrap().read(cx);
            (e.file_name(), e.buffer.to_string())
        });
        assert_eq!(name, "go.json");
        assert!(text.starts_with("// Your snippets for Go files."), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Bookmarks by keystroke: ⌘F2 in two files, then Bookmarks… lists both and goes to
    /// one; the session keeps them, and a reopened project has them back.
    #[gpui::test]
    fn bookmarks_are_listed_and_kept(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("bookmarks");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(dir.join("b.txt"), "alpha\nbeta\n").unwrap();
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
        let mark = |cx: &mut gpui::VisualTestContext, file: &str, line: usize| {
            let path = dir.join(file);
            workspace.update_in(cx, |w, window, cx| w.open_file_on(path, Some(0), window, cx));
            settle(cx);
            workspace.update_in(cx, |w, window, cx| {
                let editor = w.active_editor().unwrap();
                editor.update(cx, |e, cx| e.go_to_line(line + 1, cx));
                window.focus(&editor.focus_handle(cx));
            });
            cx.simulate_keystrokes("cmd-f2");
        };
        mark(cx, "a.txt", 2);
        mark(cx, "b.txt", 1);
        let place = |cx: &mut gpui::VisualTestContext| {
            workspace.read_with(cx, |w, cx| {
                let e = w.active_editor().unwrap().read(cx);
                (e.file_name(), e.caret_point().0)
            })
        };
        // Bookmarks…: a.txt's first, as the files sort; Enter goes there.
        workspace.update_in(cx, |w, window, cx| w.show_bookmarks(&ShowBookmarks, window, cx));
        settle(cx);
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert_eq!(place(cx), ("a.txt".into(), 2));
        // Kept with the session, and back in a reopened project.
        let session = workspace.read_with(cx, |w, cx| w.session(cx));
        let kept: Vec<Vec<usize>> = session.tabs.iter().map(|t| t.bookmarks.clone()).collect();
        assert_eq!(kept, vec![vec![2], vec![1]]);
        let root = dir.clone();
        let reopened = cx.new_window_entity(|window, cx| Workspace::new(root, window, cx));
        reopened.update_in(cx, |w, window, cx| w.restore_session(session, window, cx));
        settle(cx);
        let marks = reopened
            .read_with(cx, |w, cx| w.tabs.iter().map(|t| t.editor.read(cx).bookmarks.clone()).collect::<Vec<_>>());
        assert_eq!(marks, vec![vec![2], vec![1]]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The palettes, by keystroke: ⌘P with a place, ⌘P ":line", ⌃G, ⌘⇧O and a name, ⌘K and
    /// a command. Each row does what it says.
    #[gpui::test]
    fn palette_rows_go_where_they_say(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("palette-flows");
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
        let dir = crate::tools::test_dir("find-flows");
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
        let dir = crate::tools::test_dir("tree-drag");
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
        let dir = crate::tools::test_dir("overlay-clicks");
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
        let dir = crate::tools::test_dir("compare");
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
        let dir = crate::tools::test_dir("prose-wrap");
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
        let pattern = regex::Regex::new(r#"test_dir\(&?(?:format!\()?"([A-Za-z0-9{}-]+)""#).unwrap();
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
        let dir = crate::tools::test_dir("tab-menu-typing");
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
        let dir = crate::tools::test_dir("discard-file");
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
        let dir = crate::tools::test_dir("undo-commit-flow");
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
        let dir = crate::tools::test_dir("history-no-git");
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

    /// Copy two things, then Paste from History and ↵ on the older: pasted, as it was copied.
    #[gpui::test]
    fn an_older_copy_pastes_from_the_history(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("clipboard-history");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "alpha\nbeta\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| w.open_file(file.clone(), window, cx));
        let editor = workspace.read_with(cx, |w, _| w.active_editor().unwrap().clone());
        // Copy "alpha", then "beta" (each its whole line), then go to the end.
        cx.simulate_keystrokes("cmd-c down cmd-c cmd-down");
        workspace.update_in(cx, |w, window, cx| w.paste_from_history(&PasteFromHistory, window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("down enter");
        cx.run_until_parked();
        editor.read_with(cx, |e, _| assert_eq!(e.buffer.to_string(), "alpha\nbeta\nalpha\n"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file changed on disk under unsaved edits: saving asks before writing over it,
    /// auto-save leaves it be, and Cancel writes nothing.
    #[gpui::test]
    fn saving_over_a_file_changed_on_disk_asks(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("save-conflict");
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

    /// A project's `.null/settings.json` is used over yours in its window; another
    /// project's window in front, yours come back. Changed in Settings meanwhile, its own
    /// value is changed in its file.
    #[gpui::test]
    fn a_project_has_its_own_settings(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("project-settings");
        let _ = std::fs::remove_dir_all(&dir);
        let (ours, other) = (dir.join("ours"), dir.join("other"));
        std::fs::create_dir_all(&other).unwrap();
        let file = crate::settings::project_file(&ours);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{ \"indent_size\": 3 }\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings { indent_size: 4, ..Settings::default() });
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = ours.clone();
        let (_workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        assert_eq!(cx.update(|_, cx| cx.global::<Settings>().indent_size), 3);
        // Changed in Settings: the project's own value, in its file.
        cx.update(|_, cx| settings::update(cx, |s| s.indent_size = 5));
        assert!(std::fs::read_to_string(&file).unwrap().contains("\"indent_size\": 5"));
        // Another project in front: yours again.
        cx.update(|_, cx| crate::settings::use_project(&other, cx));
        assert_eq!(cx.update(|_, cx| cx.global::<Settings>().indent_size), 4);
        // Back, and changed by hand meanwhile: as it is now.
        std::fs::write(&file, "{ \"indent_size\": 2 }\n").unwrap();
        cx.update(|_, cx| crate::settings::use_project(&ours, cx));
        assert_eq!(cx.update(|_, cx| cx.global::<Settings>().indent_size), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A project's .vscode/settings.json is used too; changed in Settings, the change goes
    /// to .null/settings.json (made for it), never into VS Code's file.
    #[gpui::test]
    fn a_project_s_vscode_settings_are_used(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("project-vscode");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".vscode")).unwrap();
        let vscode = "{ \"editor.tabSize\": 2, \"editor.formatOnSave\": true }\n";
        std::fs::write(crate::settings::vscode_file(&dir), vscode).unwrap();
        cx.update(|cx| {
            cx.set_global(Settings { indent_size: 4, format_on_save: false, ..Settings::default() });
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (_workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let now = cx.update(|_, cx| (cx.global::<Settings>().indent_size, cx.global::<Settings>().format_on_save));
        assert_eq!(now, (2, true));
        cx.update(|_, cx| settings::update(cx, |s| s.format_on_save = false));
        assert_eq!(std::fs::read_to_string(crate::settings::vscode_file(&dir)).unwrap(), vscode, "VS Code's, untouched");
        let own = std::fs::read_to_string(crate::settings::project_file(&dir)).unwrap();
        assert!(own.contains("\"format_on_save\": false") && !own.contains("indent_size"), "{own}");
        // Read again (the watcher saw it): the same.
        cx.update(|_, cx| crate::settings::use_project(&dir, cx));
        let now = cx.update(|_, cx| (cx.global::<Settings>().indent_size, cx.global::<Settings>().format_on_save));
        assert_eq!(now, (2, false));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The status bar's place: a click on a name in it goes there; on the file, shows it in
    /// the files.
    #[gpui::test]
    fn the_status_bar_s_place_is_clicked(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("status-trail");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let file = dir.join("src/shop.py");
        std::fs::write(&file, "class Shop:\n    def checkout(self):\n        pay()\n        ship()\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings { sidebar_visible: false, ..Settings::default() });
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| w.open_file(file.clone(), window, cx));
        let editor = workspace.read_with(cx, |w, _| w.active_editor().unwrap().clone());
        editor.update(cx, |e, cx| e.set_caret_point((3, 8), cx));
        cx.run_until_parked();
        // "Shop": to its line.
        let shop = cx.debug_bounds("status-trail 0").expect("the trail is drawn").center();
        cx.simulate_click(shop, Default::default());
        cx.run_until_parked();
        assert_eq!(editor.read_with(cx, |e, _| e.caret_point().0), 0);
        // The file: shown in the files.
        let place = cx.debug_bounds("status-place").expect("the file is drawn").center();
        cx.simulate_click(place, Default::default());
        cx.run_until_parked();
        assert!(cx.update(|_, cx| cx.global::<Settings>().sidebar_visible));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Changed on disk with no word of it (another app, a watcher that missed it): saving
    /// asks, once; Save Mine then writes.
    #[gpui::test]
    fn saving_over_an_unseen_change_asks_once(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("save-unseen");
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
        std::fs::write(&file, "theirs, unseen\n").unwrap();
        editor.update(cx, |e, cx| e.save_to_disk(cx));
        cx.run_until_parked();
        assert!(cx.has_pending_prompt(), "asked before writing over it");
        cx.simulate_prompt_answer("Save Mine");
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt(), "not asked again");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "mine\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Switching where changes here would be overwritten: asked, they come along.
    #[gpui::test]
    #[cfg(unix)]
    fn switching_offers_to_bring_changes_along(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("carry-changes");
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
        let dir = crate::tools::test_dir("branch-list");
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

    /// The Outline view: the open file's functions and types, read again only when it
    /// changes; one picked takes the caret there.
    #[gpui::test]
    fn the_outline_follows_the_file(cx: &mut gpui::TestAppContext) {
        use gpui::EntityInputHandler as _;
        let dir = crate::tools::test_dir("outline");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("shop.py"),
            "class Shop:\n    def open(self):\n        pass\n\ndef main():\n    pass\n",
        )
        .unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(dir.join("shop.py"), window, cx);
            w.show_outline(&ShowOutline, window, cx);
        });
        cx.run_until_parked();
        workspace.update_in(cx, |w, window, cx| {
            let names: Vec<(String, usize)> =
                w.outline.as_ref().unwrap().2.iter().map(|i| (i.name.clone(), i.depth)).collect();
            assert_eq!(names, [("Shop".into(), 0), ("open".into(), 1), ("main".into(), 0)]);
            w.go_to_outline_row(4, window, cx);
            assert_eq!(w.active_editor().unwrap().read(cx).caret_point(), (4, 0));
            // Typed into: read again.
            w.active_editor().unwrap().update(cx, |e, cx| {
                e.set_caret_point((5, 0), cx);
                e.replace_text_in_range(None, "\ndef stop():\n    pass\n", window, cx);
            });
        });
        cx.run_until_parked();
        workspace.update_in(cx, |w, window, cx| {
            assert_eq!(w.outline.as_ref().unwrap().2.len(), 4);
            w.show_files(&ShowFiles, window, cx);
            assert!(!w.sidebar_outline);
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Files clicked once in the files take the passing tab's place; one edited, opened on
    /// purpose or double-clicked stays.
    #[gpui::test]
    fn files_looked_at_in_passing_share_a_tab(cx: &mut gpui::TestAppContext) {
        use gpui::EntityInputHandler as _;
        let dir = crate::tools::test_dir("passing-tabs");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"] {
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
        let names =
            |w: &Workspace, cx: &App| -> Vec<String> { w.tabs.iter().map(|t| t.editor.read(cx).file_name()).collect() };
        workspace.update_in(cx, |w, window, cx| {
            w.open_file(dir.join("a.txt"), window, cx);
            // Looked at: b, then c in its place.
            w.open_file_passing(dir.join("b.txt"), window, cx);
            w.open_file_passing(dir.join("c.txt"), window, cx);
            assert_eq!(names(w, cx), ["a.txt", "c.txt"]);
            assert!(w.tabs[1].passing);
            assert!(w.recently_closed.last().is_none_or(|p| !p.ends_with("b.txt")));
            // Typed into: kept, so the next one looked at gets its own tab.
            w.active_editor().unwrap().update(cx, |e, cx| e.replace_text_in_range(None, "!", window, cx));
        });
        cx.run_until_parked();
        workspace.update_in(cx, |w, window, cx| {
            assert!(!w.tabs[1].passing);
            w.open_file_passing(dir.join("d.txt"), window, cx);
            assert_eq!(names(w, cx), ["a.txt", "c.txt", "d.txt"]);
            // Opened on purpose while passing (a double-click in the files): kept.
            w.open_file(dir.join("d.txt"), window, cx);
            assert!(!w.tabs[2].passing);
            w.open_file_passing(dir.join("e.txt"), window, cx);
            assert_eq!(names(w, cx), ["a.txt", "c.txt", "d.txt", "e.txt"]);
        });
        // Turned off: every file its own tab.
        cx.update(|_, cx| cx.global_mut::<Settings>().preview_tabs = false);
        workspace.update_in(cx, |w, window, cx| {
            w.open_file_passing(dir.join("b.txt"), window, cx);
            assert_eq!(names(w, cx).len(), 5);
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Undoing all of an AI task's changes takes back only its own: a file you saved while
    /// it ran, or edited since it ended, is left as it is.
    #[gpui::test]
    fn undoing_a_task_leaves_your_own_edits(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("task-undo");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.join(name), "before\n").unwrap();
        }
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let snapshot = crate::ai_task::Snapshot::take(&dir);
        // The task changes a and c; you save b meanwhile.
        std::fs::write(dir.join("a.txt"), "by the task\n").unwrap();
        std::fs::write(dir.join("c.txt"), "by the task\n").unwrap();
        std::fs::write(dir.join("b.txt"), "by you\n").unwrap();
        workspace.update_in(cx, |w, window, cx| {
            w.ai_task = Some(AiTaskRun {
                title: "t".into(),
                state: TaskState::Running(None),
                child: Arc::default(),
                stop: Arc::default(),
                yours: HashSet::from([dir.join("b.txt")]),
                _task: None,
            });
            w.task_finished(Ok(String::new()), snapshot.changes(&dir), cx);
            // You edit c after it ended.
            std::fs::write(dir.join("c.txt"), "by the task, then you\n").unwrap();
            w.undo_all_task_changes(&UndoAllTaskChanges, window, cx);
        });
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        assert_eq!(read("a.txt"), "before\n", "the task's own: undone");
        assert_eq!(read("b.txt"), "by you\n", "saved by you meanwhile: left");
        assert_eq!(read("c.txt"), "by the task, then you\n", "edited since: left");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A theme of your own: made from the one shown, in use at once, its saved changes shown;
    /// one of Null's picked again puts it aside.
    #[gpui::test]
    fn a_theme_of_your_own(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("own-theme");
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
        let folder = crate::theme::own::folder().unwrap();
        let _ = std::fs::remove_dir_all(&folder);
        workspace.update_in(cx, |w, window, cx| {
            w.new_own_theme(&NewOwnTheme, window, cx);
            assert_eq!(cx.global::<Settings>().own_theme.as_deref(), Some("My Theme"));
            assert!(folder.join("My Theme.json").is_file());
            assert_eq!(w.active_editor().unwrap().read(cx).file_name(), "My Theme.json");
            // Its caret changed in the file, and saved: shown.
            let text = std::fs::read_to_string(folder.join("My Theme.json")).unwrap();
            let changed = text.replacen("\"caret\": \"#f2b35b\"", "\"caret\": \"#00ff00\"", 1);
            assert_ne!(text, changed, "the caret is written out");
            std::fs::write(folder.join("My Theme.json"), changed).unwrap();
            w.theme_saved("My Theme", cx);
            assert_eq!(cx.global::<Theme>().caret, crate::theme::own::colour("#00ff00").unwrap());
            // Listed in ⌘K, and one of Null's picked again puts it aside.
            assert!(w.commands(window, cx).iter().any(|c| c.label.starts_with("My Theme Theme")));
            settings::update(cx, |s| s.pick_theme(ThemeName::Moss));
            assert_eq!(cx.global::<Settings>().own_theme, None);
        });
        std::fs::remove_dir_all(&folder).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// ⌃Tab goes by when tabs were used: the one before, and on while ⌃ is held; letting go
    /// stays there, so the next ⌃Tab comes back.
    #[gpui::test]
    fn ctrl_tab_goes_by_last_used(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("switch-tab");
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
        workspace.update_in(cx, |w, window, cx| {
            let shown = |w: &Workspace, cx: &App| w.active_editor().unwrap().read(cx).file_name();
            for name in ["a.txt", "b.txt", "c.txt"] {
                w.open_file(dir.join(name), window, cx);
            }
            // Used a, b, c, then a again: by last use, a, c, b.
            w.activate(0, window, cx);
            // ⌃ held: c (used before a), then b.
            w.switch_tab(false, true, window, cx);
            assert_eq!(shown(w, cx), "c.txt");
            w.switch_tab(false, true, window, cx);
            assert_eq!(shown(w, cx), "b.txt");
            // Let go: b is the last used, so ⌃Tab alone goes back to a.
            w.settle_switch();
            w.switch_tab(false, false, window, cx);
            assert_eq!(shown(w, cx), "a.txt");
            w.switch_tab(false, false, window, cx);
            assert_eq!(shown(w, cx), "b.txt");
            // ⌃⇧Tab goes the other way round: the one used longest ago.
            w.switch_tab(true, false, window, cx);
            assert_eq!(shown(w, cx), "c.txt");
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A tab moved below: the sides one above the other, kept so; sent right, they turn.
    #[gpui::test]
    fn sides_stack_one_above_the_other(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("split-down");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt"] {
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
            for name in ["a.txt", "b.txt"] {
                w.open_file(dir.join(name), window, cx);
            }
            let b = w.active_editor().unwrap().clone();
            assert!(w.tab_menu_items(&b, cx).contains(&TabMenuItem::MoveDown));
            w.move_tab_to(1, Some(true), window, cx);
            assert!(w.is_split() && w.stacked);
            assert!(w.session(cx).stacked, "kept with the session");
            assert!(w.tab_menu_items(&b, cx).contains(&TabMenuItem::MoveBack));
            // Already below, sent right: side by side, the same tabs.
            w.move_tab_to(1, Some(true), window, cx);
            w.move_tab_to(1, Some(false), window, cx);
            assert!(w.is_split() && !w.stacked);
            // Back up: one side again.
            w.move_tab_to(0, None, window, cx);
            assert!(!w.is_split());
            assert!(!w.session(cx).stacked);
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn two_sides_open_move_and_close_back_to_one(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("split");
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
            w.move_tab_to(1, Some(false), window, cx);
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
        let dir = crate::tools::test_dir("cmd");
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
        let dir = crate::tools::test_dir("back");
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
        let dir = crate::tools::test_dir("autosave");
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
        let dir = crate::tools::test_dir("focus");
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
        let dir = crate::tools::test_dir("f8");
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
        let dir = crate::tools::test_dir("md-drop");
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
        let dir = crate::tools::test_dir("line-spacing");
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
        let dir = crate::tools::test_dir("search-shared");
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
        let dir = crate::tools::test_dir("last-edit");
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
        let dir = crate::tools::test_dir("compare-file");
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
        // A long text says about how long it takes to read.
        assert_eq!(reading_time(100), None);
        assert_eq!(reading_time(115).as_deref(), Some("1 min read"));
        assert_eq!(reading_time(1000).as_deref(), Some("4 min read"));
        let long = "word ".repeat(700);
        let (essay, cx) = cx
            .add_window_view(|_, cx| Editor::new(crate::buffer::Buffer::from_text(&long), Some("essay.md".into()), cx));
        let text = essay.update(cx, |e, _| position_label(e, 0, 0));
        assert_eq!(text, "Ln 1, Col 1 · 700 words · 3 min read");
        assert_eq!(spaced(&text), "Ln 1, Col 1\u{2002}·\u{2002}700 words\u{2002}·\u{2002}3 min read");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(999), "999");
    }

    #[test]
    fn the_servers_organize_imports_is_the_one_run() {
        use lsp_types::{CodeAction, CodeActionKind, CodeActionOrCommand, Command};
        let action = |title: &str, kind: Option<CodeActionKind>| {
            CodeActionOrCommand::CodeAction(CodeAction { title: title.into(), kind, ..Default::default() })
        };
        let picked = |list: Vec<CodeActionOrCommand>| {
            organize_action(list).map(|a| match a {
                CodeActionOrCommand::CodeAction(a) => a.title,
                CodeActionOrCommand::Command(c) => c.title,
            })
        };
        let sort = action("Sort imports", Some(CodeActionKind::SOURCE));
        let organize = action("Organize Imports", Some(CodeActionKind::new("source.organizeImports.ts")));
        assert_eq!(picked(vec![sort.clone(), organize]).as_deref(), Some("Organize Imports"));
        // One answer with no kind (a command alone): that's it.
        let command = CodeActionOrCommand::Command(Command { title: "Organize".into(), ..Default::default() });
        assert_eq!(picked(vec![command]).as_deref(), Some("Organize"));
        assert_eq!(picked(vec![]), None);
        assert_eq!(picked(vec![sort.clone(), sort.clone()]), None, "nothing that organizes imports");
        assert_eq!(picked(vec![sort]), None, "a lone action of another kind");
        let disabled = CodeActionOrCommand::CodeAction(CodeAction {
            title: "Organize Imports".into(),
            kind: Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS),
            disabled: Some(lsp_types::CodeActionDisabled { reason: "Nothing to organize".into() }),
            ..Default::default()
        });
        assert_eq!(picked(vec![disabled]), None, "a disabled one");
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
        // A name given to it wins over its folder.
        assert_eq!(Workspace::named_terminal_label(Some("server"), "ada@mac:~/code/null", 0), "server");
        assert_eq!(Workspace::named_terminal_label(None, "ada@mac:~/code/null", 0), "null");
        assert_eq!(Workspace::terminal_name("ada@mac:~/code/null"), "null");
        assert_eq!(Workspace::terminal_name("cargo run"), "cargo run");
        let labels = Workspace::distinct_labels(vec!["qa".into(), "api".into(), "qa".into(), "qa".into()]);
        assert_eq!(labels, ["qa", "api", "qa 2", "qa 3"]);
        // One you named "qa 2" keeps it; another "qa" takes the next number free.
        let labels = Workspace::distinct_labels(vec!["qa".into(), "qa".into(), "qa 2".into()]);
        assert_eq!(labels, ["qa", "qa 3", "qa 2"]);
    }

    /// The Tests view finds the project's tests; a run's output says how each did, those
    /// run from the list and, typed by hand, any of that name.
    #[gpui::test]
    fn tests_are_listed_and_their_results_read(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("tests-view");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        let code = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() {}\n    #[test]\n    fn takes() {}\n}\n";
        std::fs::write(dir.join("src/cart.rs"), code).unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| w.show_tests(&ShowTests, window, cx));
        cx.run_until_parked();
        let path = dir.join("src/cart.rs");
        let names = workspace.read_with(cx, |w, _| {
            w.tests.clone().unwrap().iter().flat_map(|f| f.tests.iter().map(|t| t.id.clone())).collect::<Vec<_>>()
        });
        assert_eq!(names, ["cart::tests::adds", "cart::tests::takes"]);
        let ran = |command: &str, output: &str| crate::terminal::Ran {
            name: "cargo".into(),
            command: command.to_string(),
            folder: dir.clone(),
            output: output.to_string(),
        };
        workspace.update(cx, |w, _| {
            let command = "cargo test -- --exact cart::tests::adds".to_string();
            w.tests_running = Some((command.clone(), vec![(path.clone(), "cart::tests::adds".into())]));
            w.test_status.insert((path.clone(), "cart::tests::adds".into()), TestStatus::Running);
            w.read_test_results(&ran(&command, "test cart::tests::adds ... FAILED\n"));
            assert_eq!(w.test_status.get(&(path.clone(), "cart::tests::adds".into())), Some(&TestStatus::Failed));
            // Typed by hand: every test it names.
            w.read_test_results(&ran("cargo test", "test cart::tests::adds ... ok\ntest cart::tests::takes ... ok\n"));
            assert_eq!(w.test_status.get(&(path.clone(), "cart::tests::adds".into())), Some(&TestStatus::Passed));
            assert_eq!(w.test_status.get(&(path.clone(), "cart::tests::takes".into())), Some(&TestStatus::Passed));
            // Run from the list but not said (it didn't build): not known.
            w.tests_running = Some(("cargo test x".into(), vec![(path.clone(), "cart::tests::takes".into())]));
            w.test_status.insert((path.clone(), "cart::tests::takes".into()), TestStatus::Running);
            w.read_test_results(&ran("cargo test x", "error[E0425]: cannot find value\n"));
            assert_eq!(w.test_status.get(&(path.clone(), "cart::tests::takes".into())), None);
        });
        // A test added and saved: listed.
        workspace.update_in(cx, |w, window, cx| w.open_file(path.clone(), window, cx));
        let editor = workspace.read_with(cx, |w, _| w.active_editor().unwrap().clone());
        let more = code.replace("    fn takes() {}\n", "    fn takes() {}\n    #[test]\n    fn more() {}\n");
        editor.update(cx, |e, cx| {
            e.restore_unsaved(&more, cx);
            e.save_to_disk(cx);
        });
        cx.run_until_parked();
        let count = workspace.read_with(cx, |w, _| w.tests.as_ref().unwrap()[0].tests.len());
        assert_eq!(count, 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A command's output in the terminal: its problems in files that exist join the list
    /// F8 and ⌘⇧M go through; the next command's replace them.
    #[gpui::test]
    fn problems_printed_in_the_terminal_are_listed(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("terminal-problems");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.c"), "int main() { return x; }\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let ran = |command: &str, output: &str| crate::terminal::Ran {
            name: command.split(' ').next().unwrap().to_string(),
            command: command.to_string(),
            folder: dir.join("src"),
            output: output.to_string(),
        };
        workspace.update(cx, |w, cx| {
            // From where it ran, the way back up included; outside the project, not listed.
            let output = "../src/main.c:1:21: error: use of undeclared identifier 'x'\ngone.c:3:1: error: not here\n/etc/hosts:1:1: error: elsewhere\n";
            w.read_reported(&ran("make", output), cx);
            let places = w.problem_places(cx);
            assert_eq!(places.len(), 1, "only files that exist: {places:?}");
            let (path, d) = &places[0];
            assert_eq!(path, &dir.join("src/main.c"));
            assert_eq!((d.range.start.line, d.range.start.character), (0, 20));
            assert_eq!(d.source.as_deref(), Some("make"));
            // Another command, or a search, leaves them be.
            w.read_reported(&ran("ls -la", "main.c\n"), cx);
            w.read_reported(&ran("grep -rn x .", "./main.c:1:int main() { return x; }\n"), cx);
            assert_eq!(w.problem_places(cx).len(), 1);
            // The same command again, clean: they go.
            w.read_reported(&ran("make", "nothing to be done\n"), cx);
            assert!(w.problem_places(cx).is_empty());
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A pinned tab goes first, stays through "Close Others" and "Close All", and is kept
    /// pinned with the session.
    #[gpui::test]
    fn pinned_tabs_stay(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("pinned-tabs");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
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
        let names =
            |w: &Workspace, cx: &App| -> Vec<String> { w.tabs.iter().map(|t| t.editor.read(cx).file_name()).collect() };
        workspace.update_in(cx, |w, window, cx| {
            for name in ["a.txt", "b.txt", "c.txt"] {
                w.open_file(dir.join(name), window, cx);
            }
            let c = w.tabs[2].editor.clone();
            assert!(w.tab_menu_items(&c, cx).contains(&TabMenuItem::Pin));
            w.tab_menu = Some(TabMenu { editor: c.clone(), position: Default::default() });
            w.run_tab_menu_item(TabMenuItem::Pin, window, cx);
            assert_eq!(names(w, cx), ["c.txt", "a.txt", "b.txt"], "first");
            assert!(w.tab_menu_items(&c, cx).contains(&TabMenuItem::Unpin));
            assert_eq!(w.active_editor(), Some(&c), "still the one shown");
            // A tab opened next to it goes after it.
            w.open_file(dir.join("d.txt"), window, cx);
            assert_eq!(names(w, cx), ["c.txt", "d.txt", "a.txt", "b.txt"]);
            let a = w.tabs[2].editor.clone();
            w.tab_menu = Some(TabMenu { editor: a, position: Default::default() });
            w.run_tab_menu_item(TabMenuItem::CloseOthers, window, cx);
            assert_eq!(names(w, cx), ["c.txt", "a.txt"], "the pinned one stays");
            let session = w.session(cx);
            assert_eq!(session.tabs.iter().map(|t| t.pinned).collect::<Vec<_>>(), [true, false]);
            w.close_all_tabs(&CloseAllTabs, window, cx);
            assert_eq!(names(w, cx), ["c.txt"]);
            // Two pinned, the first shown: a file opened goes after them, and is the one shown.
            w.open_file(dir.join("a.txt"), window, cx);
            let a = w.active_editor().unwrap().clone();
            w.set_pinned(&a, true, cx);
            let first = w.tabs[0].editor.clone();
            let ix = w.tabs.iter().position(|t| t.editor == first).unwrap();
            w.activate(ix, window, cx);
            w.open_file(dir.join("b.txt"), window, cx);
            assert_eq!(names(w, cx), ["c.txt", "a.txt", "b.txt"]);
            assert_eq!(w.active_editor().map(|e| e.read(cx).file_name()), Some("b.txt".to_string()));
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn a_tab_s_menu_closes_to_the_right_and_copies_its_path(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("tab-menu");
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
                [
                    Pin,
                    Close,
                    CloseOthers,
                    CloseToTheRight,
                    MoveRight,
                    MoveDown,
                    CopyPath,
                    CopyRelativePath,
                    Reveal,
                    OpenInTerminal,
                    OtherSide
                ]
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
        let dir = crate::tools::test_dir("rename");
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
        let dir = crate::tools::test_dir("backup");
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

    /// An error reported in a file: it and its folder stand out in the files; a warning
    /// doesn't, and once fixed the mark goes.
    #[gpui::test]
    fn files_with_errors_stand_out(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("tree-errors");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn a() {}\n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        let file = dir.join("src/a.rs");
        let report = |severity, cx: &mut gpui::VisualTestContext| {
            let file = file.clone();
            workspace.update(cx, |w, cx| {
                w.lsp.update(cx, |lsp, cx| {
                    let diagnostic = lsp_types::Diagnostic { severity, message: "x".into(), ..Default::default() };
                    lsp.set_diagnostics(file, severity.map(|_| diagnostic).into_iter().collect());
                    cx.emit(crate::lsp_store::LspEvent::DiagnosticsChanged);
                })
            });
            cx.run_until_parked();
        };
        let marked = |path: &Path, cx: &mut gpui::VisualTestContext| {
            workspace.read_with(cx, |w, cx| w.tree.read(cx).marked_for_errors(path))
        };
        report(Some(lsp_types::DiagnosticSeverity::ERROR), cx);
        assert!(marked(&file, cx) && marked(&dir.join("src"), cx));
        report(Some(lsp_types::DiagnosticSeverity::WARNING), cx);
        assert!(!marked(&file, cx), "a warning isn't an error");
        report(None, cx);
        assert!(!marked(&dir.join("src"), cx));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file dragged from the files into a Markdown file: linked where it's dropped.
    #[gpui::test]
    fn a_file_dragged_from_the_files_is_linked(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("tree-drop-link");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shot.png"), "x").unwrap();
        std::fs::write(dir.join("notes.md"), "See: \n").unwrap();
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let root = dir.clone();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(root, window, cx));
        workspace.update_in(cx, |w, window, cx| w.open_file(dir.join("notes.md"), window, cx));
        cx.run_until_parked();
        let editor = workspace.read_with(cx, |w, _| w.active_editor().unwrap().clone());
        let selector = format!("tree-row {}", dir.join("shot.png").display());
        let row = cx.debug_bounds(Box::leak(selector.into_boxed_str())).expect("the row").center();
        let target = editor.read_with(cx, |e, _| e.caret_bounds(5).expect("drawn").center());
        cx.simulate_event(gpui::MouseDownEvent {
            position: row,
            button: gpui::MouseButton::Left,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        for step in 1..=4 {
            let t = step as f32 / 4.;
            let position = gpui::point(row.x + (target.x - row.x) * t, row.y + (target.y - row.y) * t);
            cx.simulate_event(gpui::MouseMoveEvent {
                position,
                pressed_button: Some(gpui::MouseButton::Left),
                modifiers: Default::default(),
            });
        }
        cx.simulate_event(gpui::MouseUpEvent {
            position: target,
            button: gpui::MouseButton::Left,
            modifiers: Default::default(),
            click_count: 1,
        });
        cx.run_until_parked();
        editor.read_with(cx, |e, _| assert_eq!(e.buffer.to_string(), "See: ![shot](shot.png)\n"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Renaming a folder of images, then moving a guide up a level: the Markdown links to
    /// them, and the guide's own, still point where they should; the open README too.
    #[gpui::test]
    fn markdown_links_follow_renames(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("markdown-links");
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
        // Two pictures moved together (dropped at once), linked on one line of a closed file:
        // each move's changes worked out after the one before landed, so both links are right.
        std::fs::write(dir.join("b.png"), "x").unwrap();
        std::fs::write(dir.join("c.png"), "x").unwrap();
        std::fs::write(dir.join("gallery.md"), "![](b.png) ![](c.png)\n").unwrap();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        workspace.update(cx, |w, cx| {
            w.queue_rename(dir.join("b.png"), dir.join("assets/b.png"), cx);
            w.queue_rename(dir.join("c.png"), dir.join("assets/c.png"), cx);
        });
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(dir.join("gallery.md")).unwrap(), "![](assets/b.png) ![](assets/c.png)\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn a_file_moved_to_a_folder_keeps_its_tab(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("move-tab");
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
            let dir = crate::tools::test_dir(&format!("quit-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("a.txt"), "a\n").unwrap();
            dir
        };
        let (first, second) = (make("one"), make("two"));
        cx.update(|cx| {
            // Asking, rather than keeping the changes for next time.
            cx.set_global(Settings { keep_unsaved: false, ..Default::default() });
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

    /// Quitting with unsaved changes, kept for next time: no question, and they come back.
    #[gpui::test]
    fn quitting_keeps_unsaved_changes_for_next_time(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("quit-keep");
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
        let window = cx.add_window(|window, cx| Workspace::new(root, window, cx));
        window
            .update(cx, |w, window, cx| {
                w.open_file(dir.join("a.txt"), window, cx);
                w.active_editor().unwrap().update(cx, |e, cx| e.type_text_for_test(0, "kept ", cx));
                w.quit(&Quit, window, cx);
                assert!(w.quitting);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt(), "no question");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "a\n", "the file isn't saved");
        // Next time: the change is back, still unsaved.
        let root = dir.clone();
        let again = cx.add_window(|window, cx| Workspace::new(root, window, cx));
        again
            .update(cx, |w, window, cx| {
                w.restore_session(crate::session::Session::default(), window, cx);
                let e = w.active_editor().unwrap().read(cx);
                assert_eq!((e.buffer.to_string(), e.buffer.is_dirty()), ("kept a\n".to_string(), true));
            })
            .unwrap();
        crate::session::save_backups(&dir, &[]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn a_file_open_on_both_sides_stays_the_same_on_both(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("twins");
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
        // Made longer by another program (a pull): both read it, and typing on one isn't
        // replayed on the other with the read (that doubled its end).
        std::fs::write(dir.join("a.txt"), "zero\none\ntwo\nthree\nfour\nfive\n").unwrap();
        workspace.update(cx, |w, cx| w.files_changed(vec![dir.join("a.txt")], cx));
        cx.run_until_parked();
        left.update(cx, |e, cx| e.type_text_for_test(0, "!", cx));
        cx.run_until_parked();
        assert_eq!(text(cx, &right), "!zero\none\ntwo\nthree\nfour\nfive\n");
        assert_eq!(text(cx, &left), text(cx, &right));
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
