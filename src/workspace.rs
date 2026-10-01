use crate::editor::{Editor, EditorEvent, GoToDefinition, Redo, Save, SelectAll, ShowInfo, Undo};
use crate::file_tree::{FileTree, FileTreeEvent};
use crate::find_bar::{DeployFind, DeployReplace};
use crate::fonts::Fonts;
use crate::git;
use crate::lsp_store::{LspStore, Readiness};
use crate::menus::{self, Quit, ToggleFadeWhileTyping};
use crate::palette::{Command, Palette, PaletteEvent, format_keys};
use crate::project_search::{ProjectSearch, ProjectSearchEvent};
use crate::settings::{self, DEFAULT_FONT_SIZE, Settings};
use crate::theme::{Theme, ThemeName};
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
        OpenSettings,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        UseOledTheme,
        UseGraphiteTheme,
        UsePaperTheme,
        SearchProject,
        ShowFiles,
        ToggleAutocomplete,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Workspace");
    let mut keys = vec![
        KeyBinding::new("secondary-k", TogglePalette, ctx),
        KeyBinding::new("secondary-p", TogglePalette, ctx),
        KeyBinding::new("secondary-shift-p", TogglePalette, ctx),
        KeyBinding::new("secondary-o", Open, ctx),
        KeyBinding::new("secondary-w", CloseTab, ctx),
        KeyBinding::new("secondary-b", ToggleSidebar, ctx),
        KeyBinding::new("ctrl-tab", NextTab, ctx),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, ctx),
        KeyBinding::new("secondary-,", OpenSettings, ctx),
        KeyBinding::new("secondary-shift-f", SearchProject, ctx),
        KeyBinding::new("secondary-shift-e", ShowFiles, ctx),
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
const SIDEBAR_WIDTH: f32 = 240.;
/// Search results need more room than file names.
const SEARCH_SIDEBAR_WIDTH: f32 = 340.;
const SIDEBAR_SLIDE: Duration = Duration::from_millis(260);
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
        let tree = cx.new(|_| FileTree::new(root));
        let subscriptions = vec![
            cx.subscribe_in(&tree, window, |this, _, event, window, cx| match event {
                FileTreeEvent::Open(path) => this.open_file(path.clone(), window, cx),
            }),
            cx.observe_global::<Settings>(|this, cx| this.apply_settings(cx)),
            cx.observe(&lsp, |_, _, cx| cx.notify()),
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
            ready_since: None,
            branch: None,
            branch_task: None,
            focus_before_palette: None,
            _subscriptions: subscriptions,
        };
        workspace.refresh_git(cx);
        workspace
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
        window.set_window_title(&editor.file_name());
        window.focus(&editor.focus_handle(cx));
        self.tree.update(cx, |tree, cx| tree.set_active(path, cx));
        cx.notify();
    }

    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.tabs.remove(ix);
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
        let dirty: Vec<Entity<Editor>> =
            self.tabs.iter().map(|tab| tab.editor.clone()).filter(|e| e.read(cx).buffer.is_dirty()).collect();
        if dirty.is_empty() {
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
        cx.spawn_in(window, async move |_, cx| {
            let Ok(choice) = answer.await else { return };
            cx.update(|window, cx| {
                if choice == 2 {
                    return;
                }
                if choice == 0 {
                    let saved = dirty.iter().all(|editor| editor.update(cx, |editor, cx| editor.save_to_disk(cx)));
                    if !saved {
                        return;
                    }
                }
                match action {
                    CloseAction::Quit => cx.quit(),
                    CloseAction::CloseWindow => window.remove_window(),
                }
            })
            .ok();
        })
        .detach();
        false
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

    /// Every command the palette offers right now, with its shortcut.
    fn commands(&self, window: &Window, cx: &App) -> Vec<Command> {
        let settings = cx.global::<Settings>();
        let fade =
            if settings.fade_bars_while_typing { "Stop Fading Bars While Typing" } else { "Fade Bars While Typing" };
        let theme_label = |name: ThemeName| {
            let current = if settings.theme == name { " (current)" } else { "" };
            format!("Theme: {}{current}", name.label())
        };
        let mut commands: Vec<(String, Box<dyn Action>)> = vec![
            ("Open File or Folder…".into(), Box::new(Open)),
            ("Toggle Sidebar".into(), Box::new(ToggleSidebar)),
            ("Search in Project".into(), Box::new(SearchProject)),
            ("Show Files".into(), Box::new(ShowFiles)),
            (fade.into(), Box::new(ToggleFadeWhileTyping)),
            (theme_label(ThemeName::Oled), Box::new(UseOledTheme)),
            (theme_label(ThemeName::Graphite), Box::new(UseGraphiteTheme)),
            (theme_label(ThemeName::Paper), Box::new(UsePaperTheme)),
            ("Bigger Text".into(), Box::new(IncreaseFontSize)),
            ("Smaller Text".into(), Box::new(DecreaseFontSize)),
            ("Actual Size".into(), Box::new(ResetFontSize)),
            ("Open Settings File".into(), Box::new(OpenSettings)),
            (
                if settings.autocomplete { "Turn Off Autocomplete" } else { "Turn On Autocomplete" }.into(),
                Box::new(ToggleAutocomplete),
            ),
        ];
        if self.active.is_some() {
            commands.extend([
                ("Save".into(), Box::new(Save) as Box<dyn Action>),
                ("Close Tab".into(), Box::new(CloseTab)),
                ("Next Tab".into(), Box::new(NextTab)),
                ("Previous Tab".into(), Box::new(PreviousTab)),
                ("Undo".into(), Box::new(Undo)),
                ("Redo".into(), Box::new(Redo)),
                ("Select All".into(), Box::new(SelectAll)),
                ("Find…".into(), Box::new(DeployFind)),
                ("Go to Definition".into(), Box::new(GoToDefinition)),
                ("Show Info at Cursor".into(), Box::new(ShowInfo)),
                ("Replace…".into(), Box::new(DeployReplace)),
            ]);
        }
        commands.push(("Quit Null".into(), Box::new(Quit)));
        commands
            .into_iter()
            .map(|(label, action)| Command {
                label: label.into(),
                keys: window.highest_precedence_binding_for_action(action.as_ref()).map(|b| format_keys(&b)),
                action,
            })
            .collect()
    }

    fn toggle_palette(&mut self, _: &TogglePalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
            return;
        }
        self.focus_before_palette = window.focused(cx);
        let commands = self.commands(window, cx);
        let root = self.tree.read(cx).root().to_path_buf();
        let palette = cx.new(|cx| Palette::new(commands, root, cx));
        let subscription = cx.subscribe_in(&palette, window, |this, _, event, window, cx| match event {
            PaletteEvent::Dismissed => this.close_palette(window, cx),
            PaletteEvent::OpenFile(path) => {
                let path = path.clone();
                this.close_palette(window, cx);
                this.open_file(path, window, cx);
            }
            PaletteEvent::Run(action) => {
                let action = action.boxed_clone();
                this.close_palette(window, cx);
                // Run once focus is back where it was, so editor commands reach the editor.
                window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
            }
        });
        window.focus(&palette.focus_handle(cx));
        self.palette = Some((palette, subscription));
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
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
            tab.editor.update(cx, |editor, cx| editor.set_font_size(px(settings.font_size), cx));
        }
        menus::set(cx, settings.fade_bars_while_typing);
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

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
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
            Readiness::Unavailable { program } => {
                Some(div().text_color(theme.faint).child(format!("{program} isn't installed")).into_any_element())
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

#[derive(Clone, Copy)]
enum CloseAction {
    Quit,
    CloseWindow,
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
        if fading || sliding || switching {
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
                        format!("Ln {}, Col {}", line + 1, col + 1),
                        "Spaces: 4".into(),
                        editor.language_name().into(),
                    ],
                    (count(lsp_types::DiagnosticSeverity::ERROR), count(lsp_types::DiagnosticSeverity::WARNING)),
                )
            }
            None => (vec![root.display().to_string()], (0, 0)),
        };
        let tabs = self.render_tabs(cx);
        let body = match self.active_editor() {
            Some(editor) => div().size_full().child(editor.clone()),
            None => div().size_full().child(self.render_empty(cx)),
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
            .when(problems != (0, 0), |bar| {
                let (errors, warnings) = problems;
                let plural = |n: usize, word: &str| if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") };
                bar.child(
                    div()
                        .flex()
                        .gap(px(10.))
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
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_fade_while_typing))
            .on_action(cx.listener(Self::toggle_autocomplete))
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
            .child(div().flex_1().min_h_0().flex().child(sidebar_panel).child(div().flex_1().min_w_0().child(body)))
            .child(status)
            .when_some(self.palette.as_ref().map(|(p, _)| p.clone()), |root, palette| {
                root.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .justify_center()
                        .px(px(16.))
                        .pt(px(88.))
                        .bg(theme.scrim)
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| this.close_palette(window, cx)),
                        )
                        .child(palette),
                )
            })
    }
}
