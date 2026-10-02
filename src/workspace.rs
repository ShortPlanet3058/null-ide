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
use crate::settings_panel::{SettingsPanel, SettingsPanelEvent, Shortcut};
use crate::terminal::{Shell, TerminalEvent, TerminalView};
use crate::theme::{Theme, ThemeName};
use crate::welcome::{Welcome, WelcomeEvent};
use gpui::{
    Action, AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels, Point, PromptLevel, Subscription, Task, Window,
    WindowControlArea, actions, div, prelude::*, px, svg,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

actions!(
    workspace,
    [
        Open,
        CloseTab,
        ToggleSidebar,
        NextTab,
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
        ShowFiles,
        ToggleAutocomplete,
        ToggleTerminal,
        GoToLine,
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
        KeyBinding::new("secondary-shift-e", ShowFiles, ctx),
        KeyBinding::new("ctrl-`", ToggleTerminal, ctx),
        KeyBinding::new("ctrl-g", GoToLine, ctx),
        KeyBinding::new("secondary-shift-m", ShowProblems, ctx),
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
const MOD: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

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

struct Tab {
    editor: Entity<Editor>,
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
    active: Option<usize>,
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
    reindex_task: Option<Task<()>>,
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
                ProjectSearchEvent::Open { path, line, columns, query } => {
                    let (line, columns, query) = (*line, columns.clone(), query.clone());
                    this.open_file(path.clone(), window, cx);
                    if let Some(editor) = this.active_editor() {
                        editor.update(cx, |editor, cx| editor.reveal_match(line, columns, query, cx));
                    }
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
            reindex_task: None,
            key_prompt: None,
            notice: None,
            notice_task: None,
            focus_before_palette: None,
            _subscriptions: subscriptions,
        };
        workspace.refresh_git(cx);
        workspace.watch(workspace.tree.read(cx).root().to_path_buf(), cx);
        workspace.build_index(cx);
        workspace
    }

    /// Finds what the project defines, for suggestions, without slowing anything down.
    fn build_index(&mut self, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        cx.set_global(crate::project_index::ProjectContext { root: root.clone(), ..Default::default() });
        self.index_task = Some(cx.spawn(async move |_, cx| {
            let definitions = cx.background_executor().spawn(async move { crate::project_index::index(&root) }).await;
            cx.update(|cx| {
                cx.global_mut::<crate::project_index::ProjectContext>().definitions = std::sync::Arc::new(definitions)
            })
            .ok();
        }));
    }

    /// Re-reads the definitions of source files that changed.
    fn reindex(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let changed: Vec<PathBuf> = paths.iter().filter(|p| crate::project_index::is_source(p)).cloned().collect();
        if changed.is_empty() || !cx.has_global::<crate::project_index::ProjectContext>() {
            return;
        }
        let current = cx.global::<crate::project_index::ProjectContext>().definitions.clone();
        self.reindex_task = Some(cx.spawn(async move |_, cx| {
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
        let git_changed = paths.iter().any(|p| {
            p.components().any(|c| c.as_os_str() == ".git")
                && p.file_name().is_some_and(|n| n == "HEAD" || n == "index" || n == "ORIG_HEAD")
        });
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

    /// Opens a file in a tab, or switches to it if it's already open.
    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|tab| tab.editor.read(cx).path() == Some(path.as_path())) {
            self.activate(ix, window, cx);
            return;
        }
        let lsp = self.lsp.clone();
        let editor = cx.new(|cx| Editor::open(path, Some(lsp), cx));
        self.add_tab(editor, window, cx);
    }

    fn add_tab(&mut self, editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        let subscriptions = [
            cx.observe(&editor, |_, _, cx| cx.notify()),
            cx.subscribe_in(&editor, window, |this, editor, event, window, cx| match event {
                EditorEvent::Edited if cx.global::<Settings>().fade_bars_while_typing => {
                    this.chrome.set(false, FADE_IN, FADE_OUT);
                    cx.notify();
                }
                EditorEvent::Edited => {}
                // Hand edits to the settings file take effect when saved.
                EditorEvent::Saved => {
                    if editor.read(cx).path().is_some_and(|p| Some(p) == Settings::path().as_deref()) {
                        settings::reload(cx);
                    }
                    cx.notify();
                }
                EditorEvent::NeedsPath => this.ask_where_to_save(editor.clone(), window, cx),
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
        let ix = self.active.map_or(self.tabs.len(), |ix| ix + 1);
        self.tabs.insert(ix, Tab { editor, _subscriptions: subscriptions });
        self.activate(ix, window, cx);
    }

    /// Opens a folder as the project, or a file in a tab.
    fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if path.is_dir() {
            self.project_search.update(cx, |search, cx| search.set_root(path.clone(), cx));
            // Language servers work per project: restart them for the new folder.
            let root = path.clone();
            self.lsp.update(cx, |lsp, _| {
                lsp.shutdown();
                *lsp = LspStore::new(root);
            });
            for tab in &self.tabs {
                tab.editor.update(cx, |editor, cx| editor.reattach_lsp(cx));
            }
            self.refresh_git(cx);
            self.watch(path.clone(), cx);
            self.tree.update(cx, |tree, cx| tree.set_root(path, cx));
            let active = self.active_editor().and_then(|e| e.read(cx).path().map(Path::to_path_buf));
            self.tree.update(cx, |tree, cx| tree.set_active(active, cx));
        } else {
            self.open_file(path, window, cx);
        }
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(ix);
        let editor = self.tabs[ix].editor.read(cx);
        let path = editor.path().map(Path::to_path_buf);
        if let Some(path) = &path {
            self.recent_files.retain(|p| p != path);
            self.recent_files.insert(0, path.clone());
            self.recent_files.truncate(50);
        }
        window.set_window_title(&editor.file_name());
        window.focus(&editor.focus_handle(cx));
        self.share_open_files(cx);
        self.tree.update(cx, |tree, cx| tree.set_active(path, cx));
        cx.notify();
    }

    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.tabs.remove(ix);
        if let Some(path) = tab.editor.read(cx).path() {
            self.recently_closed.push(path.to_path_buf());
        }
        tab.editor.update(cx, |editor, cx| editor.release_lsp(cx));
        if self.tabs.is_empty() {
            self.active = None;
            window.set_window_title("Null");
            window.focus(&self.focus_handle);
            self.tree.update(cx, |tree, cx| tree.set_active(None, cx));
            cx.notify();
        } else {
            let active = self.active.unwrap_or(0);
            let next = if active > ix || active == self.tabs.len() { active - 1 } else { active };
            self.activate(next.min(self.tabs.len() - 1), window, cx);
        }
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.tabs[ix].editor.clone();
        if !editor.read(cx).buffer.is_dirty() {
            self.remove_tab(ix, window, cx);
            return;
        }
        let message = format!("Save changes to {}?", editor.read(cx).file_name());
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
                if choice == 2 || (choice == 0 && !editor.update(cx, |editor, cx| editor.save_to_disk(cx))) {
                    return;
                }
                if let Some(ix) = this.tabs.iter().position(|tab| tab.editor == editor) {
                    this.remove_tab(ix, window, cx);
                }
            })
            .ok();
        })
        .detach();
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
                if choice == 0 {
                    let saved = dirty.iter().all(|editor| editor.update(cx, |editor, cx| editor.save_to_disk(cx)));
                    if !saved {
                        return;
                    }
                }
                this.finish_close(action, window, cx);
            })
            .ok();
        })
        .detach();
        false
    }

    fn finish_close(&mut self, action: CloseAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            CloseAction::Quit => cx.quit(),
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

    fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_unsaved(CloseAction::Quit, window, cx) {
            cx.quit();
        }
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

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.activate((ix + 1) % self.tabs.len(), window, cx);
        }
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.activate((ix + self.tabs.len() - 1) % self.tabs.len(), window, cx);
        }
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
            (Go, "Search in Project…".into(), Box::new(SearchProject)),
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
                (Cursors, "Add Next Occurrence".into(), Box::new(crate::editor::AddNextOccurrence)),
                (Cursors, "Select All Occurrences".into(), Box::new(crate::editor::SelectAllOccurrences)),
                (Cursors, "Add Cursor Above".into(), Box::new(crate::editor::AddCursorAbove)),
                (Cursors, "Add Cursor Below".into(), Box::new(crate::editor::AddCursorBelow)),
                (Go, "Go to Line…".into(), Box::new(GoToLine)),
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
        }
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
        self.palette = None;
        self.open_palette_with(PaletteKind::Locations, Some(title), locations, window, cx);
    }

    fn open_palette_with(
        &mut self,
        kind: PaletteKind,
        title: Option<String>,
        locations: Vec<crate::palette::Location>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            commands: if kind == PaletteKind::Quick { self.commands(window, cx) } else { Vec::new() },
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
            PaletteEvent::OpenFile(path) => {
                let path = path.clone();
                this.close_palette(window, cx);
                this.open_file(path, window, cx);
            }
            PaletteEvent::OpenLocation(path, position) => {
                let (path, position) = (path.clone(), *position);
                this.close_palette(window, cx);
                this.open_file(path, window, cx);
                if let Some(editor) = this.active_editor() {
                    let range = lsp_types::Range { start: position, end: position };
                    editor.update(cx, |editor, cx| editor.select_lsp_range(range, cx));
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

    fn ask_ai(&mut self, _: &AskAi, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            editor.update(cx, |editor, cx| editor.ask_inline(window, cx));
        }
    }

    fn toggle_ai(&mut self, _: &ToggleAi, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |s| s.ai.enabled = !s.ai.enabled);
        let ai = &cx.global::<Settings>().ai;
        if ai.enabled && ai.provider == ProviderId::Off {
            self.show_notice("AI is on. Choose where answers come from in Settings → AI (⌘,).".into(), cx);
        }
    }

    fn toggle_palette(&mut self, _: &TogglePalette, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteKind::Files, window, cx);
    }

    fn go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(PaletteKind::Line, window, cx);
    }

    /// Closes whichever floating layer is open (the palette or the Settings window)
    /// and puts focus back where it was.
    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            div()
                .id(id)
                .px(px(8.))
                .py(px(3.))
                .rounded(px(6.))
                .text_size(px(12.))
                .text_color(if on { theme.foreground } else { theme.faint })
                .when(on, |t| t.bg(theme.hairline))
                .when(!on, |t| t.hover(|s| s.text_color(theme.muted)))
                .child(label)
        };
        div()
            .flex()
            .gap(px(4.))
            .px(px(10.))
            .pt(px(10.))
            .child(
                tab("files", "Files", !searching)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.show_files(&ShowFiles, window, cx))),
            )
            .child(tab("search", "Search", searching).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.search_project(&SearchProject, window, cx)),
            ))
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
                Err(err) => return eprintln!("null: couldn't start a terminal: {err}"),
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
            .text_size(px(12.))
            .text_color(theme.muted)
            .child(div().text_color(theme.foreground).child("Terminal"))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_color(theme.faint).child(title))
            .child(
                div()
                    .id("close-terminal")
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .hover(|s| s.bg(theme.hairline))
                    .child(svg().path("icons/x.svg").size(px(12.)).text_color(theme.muted))
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
        let panel = cx.new(|cx| SettingsPanel::new(shortcuts, lsp, cx));
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

    fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>();
        div()
            .id("tabs")
            .flex()
            .gap(px(2.))
            .min_w_0()
            .overflow_x_scroll()
            .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
                let editor = tab.editor.read(cx);
                let active = self.active == Some(ix);
                let dirty = editor.buffer.is_dirty();
                let group = format!("tab-{ix}");
                let close = div()
                    .id(("close", ix))
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .text_size(px(14.))
                    .text_color(if active || dirty { theme.muted } else { gpui::transparent_black() })
                    .group_hover(group.clone(), |s| s.text_color(theme.muted))
                    .hover(|s| s.bg(theme.faint.opacity(0.4)).text_color(theme.foreground))
                    .child(if dirty { "●" } else { "×" })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.close_tab_at(ix, window, cx);
                    }));
                div()
                    .id(("tab", ix))
                    .group(group)
                    .h(px(28.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(12.))
                    .pr(px(6.))
                    .rounded(px(7.))
                    .text_size(px(12.5))
                    .text_color(if active { theme.foreground } else { theme.muted })
                    .when(active, |tab| tab.bg(theme.hairline))
                    .when(!active, |tab| tab.hover(|s| s.bg(theme.hairline.opacity(0.6))))
                    .child(editor.file_name())
                    .child(close)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.activate(ix, window, cx)))
            }))
            .into_any_element()
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>();
        let hint = |keys: String, label: &'static str| {
            div()
                .flex()
                .gap(px(12.))
                .child(div().w(px(56.)).text_right().text_color(theme.muted).child(keys))
                .child(div().text_color(theme.faint).child(label))
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .text_size(px(13.))
            .child(hint(format!("{MOD}O"), "Open a file or folder"))
            .child(hint(format!("{MOD}B"), "Show or hide the files"))
            .into_any_element()
    }
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
                        path.unwrap_or_else(|| "untitled".into()),
                        match editor.extra.len() {
                            0 => format!("Ln {}, Col {}", line + 1, col + 1),
                            n => format!("{} cursors · Esc for one", n + 1),
                        },
                        "Spaces: 4".into(),
                        editor.language_name().into(),
                    ],
                    (count(lsp_types::DiagnosticSeverity::ERROR), count(lsp_types::DiagnosticSeverity::WARNING)),
                )
            }
            None => (vec![root.display().to_string()], (0, 0)),
        };
        let tabs = self.render_tabs(cx);
        let terminal_panel = self.render_terminal_panel(TERMINAL_HEIGHT * terminal_shown, cx);
        let body = match self.active_editor() {
            Some(editor) => div().size_full().child(editor.clone()),
            None => div().size_full().child(self.render_empty(cx)),
        };
        // One floating layer at a time: the palette, an AI answer, or the key prompt.
        let overlay: Option<AnyElement> = if let Some((welcome, _)) = &self.welcome {
            Some(welcome.clone().into_any_element())
        } else if let Some((palette, _)) = &self.palette {
            Some(palette.clone().into_any_element())
        } else if let Some((panel, _)) = &self.settings_panel {
            Some(panel.clone().into_any_element())
        } else {
            self.key_prompt.as_ref().map(|(prompt, _)| prompt.clone().into_any_element())
        };
        let ai_provider = cx.global::<Settings>().ai.active();
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
            .child(tabs);

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
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(5.))
                    .child(svg().path("icons/branch.svg").size(px(13.)).text_color(theme.muted))
                    .child(branch)
            }))
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().children(items.next()))
            .children(lsp_status)
            .when(ai_provider != ProviderId::Off, |bar| {
                bar.child(
                    div()
                        .id("ai-status")
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_color(theme.muted)
                        .child(div().size(px(6.)).rounded_full().bg(theme.caret.opacity(0.6)))
                        .child(format!("AI · {}", ai_provider.label()))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.open_settings(&OpenSettings, window, cx)
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
                        .gap(px(10.))
                        .cursor_pointer()
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
            .children(items);

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
                                this.key_prompt = None;
                                this.close_palette(window, cx);
                            }),
                        )
                        .child(layer),
                )
            })
            .children(self.notice.as_ref().map(|(message, _)| {
                div().absolute().bottom(px(44.)).left_0().w_full().flex().justify_center().child(
                    div()
                        .px(px(14.))
                        .py(px(8.))
                        .rounded(px(9.))
                        .bg(theme.raised)
                        .border_1()
                        .border_color(theme.hairline)
                        .shadow_lg()
                        .text_size(px(13.))
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
    fn renamed_paths_never_gain_a_trailing_slash() {
        let (from, to) = (Path::new("/p/old.py"), Path::new("/p/test.py"));
        assert_eq!(moved_path(from, from, to).unwrap(), PathBuf::from("/p/test.py"));
        assert!(!moved_path(from, from, to).unwrap().to_string_lossy().ends_with('/'));
        let (dir, new_dir) = (Path::new("/p/src"), Path::new("/p/lib"));
        assert_eq!(moved_path(Path::new("/p/src/a/b.rs"), dir, new_dir).unwrap(), PathBuf::from("/p/lib/a/b.rs"));
        assert_eq!(moved_path(Path::new("/p/other.rs"), dir, new_dir), None);
    }
}
