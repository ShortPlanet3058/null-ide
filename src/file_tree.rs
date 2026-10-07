use crate::fs_ops;
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use crate::ui;
use gpui::{
    App, ClickEvent, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, KeyContext,
    MouseButton, MouseDownEvent, Pixels, Point, ScrollStrategy, SharedString, Subscription, Transformation,
    UniformListScrollHandle, Window, actions, anchored, deferred, div, prelude::*, px, radians, svg, uniform_list,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

actions!(
    file_tree,
    [
        NewFile,
        NewFolder,
        Rename,
        Duplicate,
        Trash,
        CopyPath,
        CopyRelativePath,
        Reveal,
        CollapseAll,
        SelectNext,
        SelectPrevious,
        ExpandOrOpen,
        Collapse,
        Activate,
        CommitEdit,
        CancelEdit,
        MenuNext,
        MenuPrevious,
        MenuConfirm,
        MenuClose,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let tree = Some("FileTree");
    let menu = Some("FileTree && menu_open");
    let edit = Some("TreeEdit");
    let mut keys = vec![
        KeyBinding::new("down", SelectNext, tree),
        KeyBinding::new("up", SelectPrevious, tree),
        KeyBinding::new("right", ExpandOrOpen, tree),
        KeyBinding::new("left", Collapse, tree),
        KeyBinding::new("f2", Rename, tree),
        KeyBinding::new("enter", CommitEdit, edit),
        KeyBinding::new("escape", CancelEdit, edit),
        // Arrows stay in the name being typed instead of moving through the tree.
        KeyBinding::new("up", gpui::NoAction {}, edit),
        KeyBinding::new("down", gpui::NoAction {}, edit),
    ];
    if cfg!(target_os = "macos") {
        // As in Finder: Return renames, Cmd+Down opens, Cmd+Backspace moves to the Trash.
        keys.extend([
            KeyBinding::new("enter", Rename, tree),
            KeyBinding::new("cmd-down", Activate, tree),
            KeyBinding::new("cmd-backspace", Trash, tree),
            KeyBinding::new("delete", Trash, tree),
        ]);
    } else {
        keys.extend([KeyBinding::new("enter", Activate, tree), KeyBinding::new("delete", Trash, tree)]);
    }
    // Last, so that with the menu open its keys win over the tree's (Return renames otherwise).
    keys.extend([
        KeyBinding::new("down", MenuNext, menu),
        KeyBinding::new("up", MenuPrevious, menu),
        KeyBinding::new("enter", MenuConfirm, menu),
        KeyBinding::new("escape", MenuClose, menu),
    ]);
    cx.bind_keys(keys);
}

pub const ROW_HEIGHT: f32 = ui::ROW;
/// Width of the sidebar the tree is drawn in.
pub const TREE_WIDTH: f32 = 240.;
const INDENT: f32 = 14.;
const CHEVRON: &str = "icons/chevron-right.svg";
/// How long a folder's arrow takes to turn when it opens or closes.
const TURN: Duration = Duration::from_millis(160);
/// Never worth showing in a project tree.
const ALWAYS_HIDDEN: &[&str] = &[".git", ".DS_Store"];
pub const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else if cfg!(target_os = "windows") {
    "Reveal in File Explorer"
} else {
    "Open Containing Folder"
};

/// A file or folder being dragged to another folder.
#[derive(Clone)]
pub struct DraggedEntry {
    path: PathBuf,
    name: SharedString,
}

impl DraggedEntry {
    /// The file or folder being dragged.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// What follows the pointer while dragging: the name, as a small pill.
struct EntryGhost {
    name: SharedString,
}

impl Render for EntryGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        div()
            .px(px(10.))
            .py(px(3.))
            .rounded(px(ui::R_CONTROL))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.line_strong)
            .shadow_md()
            .text_size(px(ui::T_MD))
            .text_color(theme.foreground)
            .font_family(cx.global::<crate::fonts::Fonts>().ui.clone())
            .child(self.name.clone())
    }
}

pub enum FileTreeEvent {
    /// Open and move to the file (double-click, Enter).
    Open(PathBuf),
    /// Show the file but keep the keyboard in the tree (single click, arrows),
    /// so shortcuts like rename and delete act on the tree.
    Preview(PathBuf),
    Created(PathBuf),
    Renamed {
        from: PathBuf,
        to: PathBuf,
    },
    /// A file or folder is to be renamed: the workspace lets language servers update the
    /// code that names it, then calls [`FileTree::finish_rename`].
    RenameRequested {
        from: PathBuf,
        to: PathBuf,
    },
    Trashed(PathBuf),
    /// A new terminal, started in this folder.
    OpenTerminal(PathBuf),
    /// Find in Folder…: project search, in this folder only.
    FindInFolder(PathBuf),
    /// Back to the last commit, this file (after asking).
    DiscardChanges(PathBuf, crate::git::FileStatus),
    /// Something to tell the person, like a failed operation.
    Notice(String),
}

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    name: SharedString,
    is_dir: bool,
    /// Matched by a `.gitignore`. Still shown, but dimmed: files like `.env`
    /// are ignored by git and still worth opening.
    ignored: bool,
}

enum RowKind {
    Entry(Entry),
    /// Where the name of a new file or folder is typed.
    NewItem {
        is_dir: bool,
    },
}

struct Row {
    kind: RowKind,
    depth: usize,
    expanded: bool,
}

#[derive(Clone, PartialEq)]
enum EditKind {
    NewFile { dir: PathBuf },
    NewFolder { dir: PathBuf },
    Rename { path: PathBuf },
}

struct Edit {
    kind: EditKind,
    input: Entity<TextInput>,
    error: Option<String>,
    _subscription: Subscription,
}

/// Where Open in Terminal starts: in a folder, the folder; at a file, its folder; on the
/// empty space below the files, the project's.
fn terminal_dir(target: Option<(&Path, bool)>, root: &Path) -> PathBuf {
    match target {
        Some((path, true)) => path.to_path_buf(),
        Some((path, false)) => path.parent().unwrap_or(root).to_path_buf(),
        None => root.to_path_buf(),
    }
}

/// Whether the file is a page or a picture a browser shows (HTML, SVG): it can open there.
pub fn opens_in_browser(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    matches!(ext.as_str(), "html" | "htm" | "xhtml" | "svg")
}

/// Opens the file in the default browser: the app web addresses open in, not the one for
/// the file's kind (an SVG's is Preview; an HTML file's can be an editor, Null itself).
pub fn open_in_browser(path: &Path, cx: &mut App) {
    #[cfg(target_os = "macos")]
    if let Some(browser) = default_browser() {
        std::process::Command::new("open").arg("-a").arg(browser).arg(path).spawn().ok();
        return;
    }
    if let Ok(url) = url::Url::from_file_path(path) {
        cx.open_url(url.as_str());
    }
}

/// The app that opens web addresses.
#[cfg(target_os = "macos")]
fn default_browser() -> Option<std::path::PathBuf> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    let web = NSURL::URLWithString(&NSString::from_str("https://example.com"))?;
    let app = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&web)?;
    Some(std::path::PathBuf::from(app.path()?.to_string()))
}

#[derive(Clone, Copy, PartialEq)]
enum MenuItem {
    Open,
    OpenInBrowser,
    NewFile,
    NewFolder,
    Rename,
    Duplicate,
    CopyPath,
    CopyRelativePath,
    Reveal,
    OpenInTerminal,
    FindInFolder,
    DiscardChanges,
    CollapseAll,
    Trash,
}

impl MenuItem {
    fn label(self) -> &'static str {
        match self {
            MenuItem::Open => "Open",
            MenuItem::OpenInBrowser => "Open in Browser",
            MenuItem::NewFile => "New File…",
            MenuItem::NewFolder => "New Folder…",
            MenuItem::Rename => "Rename…",
            MenuItem::Duplicate => "Duplicate",
            MenuItem::CopyPath => "Copy Path",
            MenuItem::CopyRelativePath => "Copy Relative Path",
            MenuItem::Reveal => REVEAL_LABEL,
            MenuItem::OpenInTerminal => "Open in Terminal",
            MenuItem::FindInFolder => "Find in Folder…",
            MenuItem::DiscardChanges => "Discard Changes…",
            MenuItem::CollapseAll => "Collapse All Folders",
            MenuItem::Trash => "Move to Trash",
        }
    }

    fn keys(self) -> Option<&'static str> {
        match self {
            MenuItem::Rename => Some(if cfg!(target_os = "macos") { "↵" } else { "F2" }),
            MenuItem::Trash => Some(if cfg!(target_os = "macos") { "⌘⌫" } else { "Del" }),
            _ => None,
        }
    }

    /// A line is drawn above these, to group the menu.
    fn starts_group(self) -> bool {
        matches!(self, MenuItem::Rename | MenuItem::CopyPath | MenuItem::DiscardChanges | MenuItem::Trash)
    }
}

struct Menu {
    /// What was right-clicked. None for the empty space below the files (the project root).
    target: Option<Entry>,
    position: Point<Pixels>,
    items: Vec<MenuItem>,
    selected: Option<usize>,
}

/// The project's files, folders first. Folders are read when first expanded.
pub struct FileTree {
    focus_handle: FocusHandle,
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    /// When each folder was last opened or closed, to animate its arrow.
    toggled_at: HashMap<PathBuf, Instant>,
    children: HashMap<PathBuf, Vec<Entry>>,
    rows: Vec<Row>,
    /// The open file, highlighted.
    active: Option<PathBuf>,
    /// What keyboard actions and the menu apply to.
    selected: Option<PathBuf>,
    edit: Option<Edit>,
    menu: Option<Menu>,
    scroll: UniformListScrollHandle,
    /// Files changed since the last commit, and the folders holding any.
    git: HashMap<PathBuf, crate::git::FileStatus>,
    git_folders: HashSet<PathBuf>,
    /// Files a language server finds errors in, and the folders holding any.
    errors: HashSet<PathBuf>,
    error_folders: HashSet<PathBuf>,
    /// Files and folders aren't renamed straight away: the workspace is asked first (see
    /// [`FileTreeEvent::RenameRequested`]).
    pub ask_before_renaming: bool,
    /// What's been typed to go to a file by its name (as in the Finder), and when.
    typed: (String, Instant),
}

impl EventEmitter<FileTreeEvent> for FileTree {}

/// A pause this long starts a new name.
const TYPING_PAUSE: std::time::Duration = std::time::Duration::from_secs(1);

impl FileTree {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut tree = Self {
            focus_handle: cx.focus_handle(),
            expanded: HashSet::from([root.clone()]),
            toggled_at: HashMap::new(),
            root,
            children: HashMap::new(),
            rows: Vec::new(),
            active: None,
            selected: None,
            edit: None,
            menu: None,
            ask_before_renaming: false,
            scroll: UniformListScrollHandle::new(),
            git: HashMap::new(),
            git_folders: HashSet::new(),
            errors: HashSet::new(),
            error_folders: HashSet::new(),
            typed: (String::new(), Instant::now()),
        };
        tree.rebuild();
        tree
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folders expanded below the root, for the session.
    pub fn expanded_folders(&self) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = self.expanded.iter().filter(|p| **p != self.root).cloned().collect();
        folders.sort();
        folders
    }

    /// Expands folders again, as they were last time.
    pub fn expand_folders(&mut self, folders: &[PathBuf], cx: &mut Context<Self>) {
        self.expanded.extend(folders.iter().filter(|p| p.starts_with(&self.root) && p.is_dir()).cloned());
        self.rebuild();
        cx.notify();
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.expanded = HashSet::from([root.clone()]);
        self.root = root;
        self.children.clear();
        self.active = None;
        self.selected = None;
        self.edit = None;
        self.menu = None;
        self.rebuild();
        cx.notify();
    }

    /// Highlights `path` and expands its folders so it's visible.
    pub fn set_active(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if let Some(path) = &path {
            self.expand_to(path);
            self.selected = Some(path.clone());
        }
        self.active = path;
        self.rebuild();
        self.reveal_selected();
        cx.notify();
    }

    fn expand_to(&mut self, path: &Path) {
        if let Ok(relative) = path.strip_prefix(&self.root) {
            let mut dir = self.root.clone();
            for part in relative.parent().into_iter().flat_map(Path::components) {
                dir.push(part);
                self.expanded.insert(dir.clone());
            }
        }
    }

    /// Re-reads the folders affected by changes on disk (from the file watcher).
    pub fn refresh(&mut self, changed: &[PathBuf], cx: &mut Context<Self>) {
        let mut stale = false;
        for path in changed {
            for dir in [Some(path.as_path()), path.parent()].into_iter().flatten() {
                stale |= self.children.remove(dir).is_some();
            }
        }
        if stale {
            self.rebuild();
            cx.notify();
        }
    }

    /// The folder's entries, folders first. Everything inside an ignored folder is ignored
    /// (dimmed) too: `node_modules/x` as well as `node_modules`.
    fn read_dir(dir: &Path, inside_ignored: bool) -> Vec<Entry> {
        let visible: HashSet<PathBuf> = ignore::WalkBuilder::new(dir)
            .max_depth(Some(1))
            .hidden(false)
            .build()
            .filter_map(Result::ok)
            .filter(|e| e.depth() == 1)
            .map(|e| e.into_path())
            .collect();
        let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut entries: Vec<Entry> = read
            .filter_map(Result::ok)
            .filter(|e| !ALWAYS_HIDDEN.contains(&e.file_name().to_string_lossy().as_ref()))
            .map(|e| {
                let path = e.path();
                // The entry's own type, without another look at the disk (links: what they point to).
                let is_dir = e.file_type().is_ok_and(|t| t.is_dir() || (t.is_symlink() && path.is_dir()));
                Entry {
                    name: e.file_name().to_string_lossy().into_owned().into(),
                    is_dir,
                    ignored: inside_ignored || !visible.contains(&path),
                    path,
                }
            })
            .collect();
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        entries
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        let root = self.root.clone();
        self.push_new_item_row(&root, 0);
        self.push_children(&root, 0);
    }

    fn push_new_item_row(&mut self, dir: &Path, depth: usize) {
        match self.edit.as_ref().map(|e| &e.kind) {
            Some(EditKind::NewFile { dir: d }) if d == dir => {
                self.rows.push(Row { kind: RowKind::NewItem { is_dir: false }, depth, expanded: false })
            }
            Some(EditKind::NewFolder { dir: d }) if d == dir => {
                self.rows.push(Row { kind: RowKind::NewItem { is_dir: true }, depth, expanded: false })
            }
            _ => {}
        }
    }

    fn push_children(&mut self, dir: &Path, depth: usize) {
        self.push_children_of(dir, depth, false);
    }

    fn push_children_of(&mut self, dir: &Path, depth: usize, inside_ignored: bool) {
        let entries =
            self.children.entry(dir.to_path_buf()).or_insert_with(|| Self::read_dir(dir, inside_ignored)).clone();
        for entry in entries {
            let expanded = entry.is_dir && self.expanded.contains(&entry.path);
            let (path, ignored) = (entry.path.clone(), entry.ignored);
            self.rows.push(Row { kind: RowKind::Entry(entry), depth, expanded });
            if expanded {
                self.push_new_item_row(&path, depth + 1);
                self.push_children_of(&path, depth + 1, ignored);
            }
        }
    }

    /// Scrolls the selected row into view, the least needed.
    fn reveal_selected(&mut self) {
        if let Some(ix) = self.selected_ix() {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
    }

    fn entry_at(&self, ix: usize) -> Option<&Entry> {
        match &self.rows.get(ix)?.kind {
            RowKind::Entry(entry) => Some(entry),
            RowKind::NewItem { .. } => None,
        }
    }

    fn selected_ix(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.rows.iter().position(|r| matches!(&r.kind, RowKind::Entry(e) if &e.path == selected))
    }

    fn selected_entry(&self) -> Option<Entry> {
        self.selected_ix().and_then(|ix| self.entry_at(ix)).cloned()
    }

    /// How far a folder's arrow has turned: 0 points right (closed), 1 points down (open).
    /// Also says whether it's still turning.
    fn chevron_turn(&self, path: &Path, expanded: bool) -> (f32, bool) {
        let target = if expanded { 1. } else { 0. };
        let Some(at) = self.toggled_at.get(path) else { return (target, false) };
        let t = (at.elapsed().as_secs_f32() / TURN.as_secs_f32()).min(1.);
        let eased = 1. - (1. - t).powi(3);
        (1. - target + (2. * target - 1.) * eased, t < 1.)
    }

    fn toggle(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
        self.toggled_at.insert(path.to_path_buf(), Instant::now());
        self.rebuild();
        cx.notify();
    }

    fn click(&mut self, ix: usize, click_count: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.entry_at(ix).cloned() else { return };
        self.selected = Some(entry.path.clone());
        self.menu = None;
        window.focus(&self.focus_handle);
        if entry.is_dir {
            if click_count == 1 {
                self.toggle(&entry.path, cx);
            }
        } else if click_count >= 2 {
            cx.emit(FileTreeEvent::Open(entry.path));
        } else {
            cx.emit(FileTreeEvent::Preview(entry.path));
        }
        cx.notify();
    }

    // ---------- keyboard ----------

    /// Letters typed with the files focused go to the first one shown whose name starts with
    /// them (from the one selected on), as in the Finder; a pause starts over.
    fn type_to_select(&mut self, event: &gpui::KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keys = &event.keystroke;
        if self.edit.is_some() || self.menu.is_some() || keys.modifiers.control || keys.modifiers.platform {
            return;
        }
        let Some(typed) = keys.key_char.as_deref().filter(|t| t.chars().all(|c| !c.is_control()) && *t != " ") else {
            return;
        };
        let (so_far, at) = &mut self.typed;
        if at.elapsed() > TYPING_PAUSE {
            so_far.clear();
        }
        so_far.push_str(&typed.to_lowercase());
        *at = Instant::now();
        let wanted = so_far.clone();
        // The same letter again goes on to the next name with it (as in the Finder).
        let again = wanted.len() > 1 && wanted.chars().all(|c| wanted.starts_with(c));
        let prefix = if again { &wanted[..wanted.chars().next().map_or(1, char::len_utf8)] } else { &wanted[..] };
        let current = self.selected_ix().unwrap_or(0);
        let start = if again { current + 1 } else { current };
        let rows = self.rows.len();
        let found = (0..rows).map(|i| (start + i) % rows.max(1)).find(|&ix| {
            self.entry_at(ix).is_some_and(|e| {
                e.path.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with(prefix))
            })
        });
        if let Some(ix) = found {
            self.selected = self.entry_at(ix).map(|e| e.path.clone());
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn select_offset(&mut self, delta: isize, cx: &mut Context<Self>) {
        let entries: Vec<usize> = (0..self.rows.len()).filter(|&i| self.entry_at(i).is_some()).collect();
        if entries.is_empty() {
            return;
        }
        let current = self.selected_ix().and_then(|ix| entries.iter().position(|&i| i == ix));
        let next = match current {
            Some(i) => (i as isize + delta).clamp(0, entries.len() as isize - 1) as usize,
            None => 0,
        };
        let ix = entries[next];
        self.selected = self.entry_at(ix).map(|e| e.path.clone());
        // Moving down keeps the row at the bottom edge, moving up at the top: never a jump.
        self.scroll.scroll_to_item(ix, if delta > 0 { ScrollStrategy::Bottom } else { ScrollStrategy::Top });
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.select_offset(1, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.select_offset(-1, cx);
    }

    fn expand_or_open(&mut self, _: &ExpandOrOpen, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else { return };
        if entry.is_dir {
            if !self.expanded.contains(&entry.path) {
                self.toggle(&entry.path, cx);
            }
        } else {
            cx.emit(FileTreeEvent::Preview(entry.path));
        }
    }

    fn collapse(&mut self, _: &Collapse, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else { return };
        if entry.is_dir && self.expanded.contains(&entry.path) {
            self.toggle(&entry.path, cx);
        } else if let Some(parent) = entry.path.parent().filter(|p| *p != self.root) {
            // On a file or closed folder, Left goes up to the parent folder.
            self.selected = Some(parent.to_path_buf());
            self.reveal_selected();
            cx.notify();
        }
    }

    fn activate(&mut self, _: &Activate, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else { return };
        if entry.is_dir {
            self.toggle(&entry.path, cx);
        } else {
            cx.emit(FileTreeEvent::Open(entry.path));
        }
    }

    // ---------- creating and renaming ----------

    /// The folder new items go into for a target: itself if it's a folder, else its folder.
    fn dir_for(&self, target: Option<&Entry>) -> PathBuf {
        match target {
            Some(e) if e.is_dir => e.path.clone(),
            Some(e) => e.path.parent().map(Path::to_path_buf).unwrap_or_else(|| self.root.clone()),
            None => self.root.clone(),
        }
    }

    fn start_edit(&mut self, kind: EditKind, initial: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Name", cx);
            input.set_text(initial, cx);
            // For a rename, select the name without its extension, like Finder does.
            if let Some(dot) = initial.rfind('.').filter(|&i| i > 0) {
                input.select_range(0..dot, cx);
            }
            input
        });
        let subscription = cx.subscribe(&input, |this, _, TextInputEvent::Changed, cx| {
            if let Some(edit) = &mut this.edit {
                edit.error = None;
                cx.notify();
            }
        });
        if let EditKind::NewFile { dir } | EditKind::NewFolder { dir } = &kind
            && *dir != self.root
        {
            self.expanded.insert(dir.clone());
        }
        self.edit = Some(Edit { kind, input: input.clone(), error: None, _subscription: subscription });
        self.rebuild();
        if let Some(ix) =
            self.rows.iter().position(|r| matches!(r.kind, RowKind::NewItem { .. })).or(self.selected_ix())
        {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        }
        window.focus(&input.focus_handle(cx));
        cx.notify();
    }

    pub fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.dir_for(self.selected_entry().as_ref());
        self.start_edit(EditKind::NewFile { dir }, "", window, cx);
    }

    pub fn new_folder(&mut self, _: &NewFolder, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.dir_for(self.selected_entry().as_ref());
        self.start_edit(EditKind::NewFolder { dir }, "", window, cx);
    }

    pub fn rename(&mut self, _: &Rename, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.selected_entry() {
            self.start_edit(EditKind::Rename { path: entry.path }, &entry.name, window, cx);
        }
    }

    fn commit_edit(&mut self, _: &CommitEdit, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.edit else { return };
        let name = edit.input.read(cx).text().to_string();
        let result = match &edit.kind {
            EditKind::NewFile { dir } => fs_ops::create_file(dir, &name).map(|p| (None, p)),
            EditKind::NewFolder { dir } => fs_ops::create_dir(dir, &name).map(|p| (None, p)),
            // A rename waits for the workspace (the code naming it may need updating first).
            EditKind::Rename { path } if self.ask_before_renaming => {
                let from = path.clone();
                match fs_ops::rename_target(&from, &name) {
                    Ok(to) => {
                        self.edit = None;
                        self.rebuild();
                        window.focus(&self.focus_handle);
                        if to != from {
                            cx.emit(FileTreeEvent::RenameRequested { from, to });
                        }
                    }
                    Err(error) => {
                        if let Some(edit) = &mut self.edit {
                            edit.error = Some(error);
                        }
                    }
                }
                return cx.notify();
            }
            EditKind::Rename { path } => fs_ops::rename(path, &name).map(|p| (Some(path.clone()), p)),
        };
        match result {
            Ok((from, path)) => {
                let is_file = path.is_file();
                self.edit = None;
                self.children.clear();
                self.expand_to(&path);
                self.selected = Some(path.clone());
                self.rebuild();
                self.reveal_selected();
                window.focus(&self.focus_handle);
                match from {
                    Some(from) if from != path => cx.emit(FileTreeEvent::Renamed { from, to: path }),
                    Some(_) => {}
                    None if is_file => cx.emit(FileTreeEvent::Created(path)),
                    None => {}
                }
            }
            Err(error) => {
                if let Some(edit) = &mut self.edit {
                    edit.error = Some(error);
                }
            }
        }
        cx.notify();
    }

    /// A file or folder dropped on a folder: it moves there (through the workspace, which
    /// lets language servers update the code naming it).
    fn drop_into(&mut self, dragged: &DraggedEntry, dir: PathBuf, cx: &mut Context<Self>) {
        if dragged.path.parent() == Some(dir.as_path()) {
            return;
        }
        match fs_ops::move_target(&dragged.path, &dir) {
            Err(message) => cx.emit(FileTreeEvent::Notice(message)),
            Ok(to) if self.ask_before_renaming => {
                cx.emit(FileTreeEvent::RenameRequested { from: dragged.path.clone(), to })
            }
            Ok(to) => {
                if let Err(message) = self.finish_rename(&dragged.path, &to, cx) {
                    cx.emit(FileTreeEvent::Notice(message));
                }
            }
        }
    }

    /// Files and folders dropped from the Finder on a folder: copied into it (beside what
    /// has the same name, never over it), the last one chosen.
    fn copy_in(&mut self, paths: &[PathBuf], dir: &Path, cx: &mut Context<Self>) {
        let mut problems = Vec::new();
        for path in paths {
            match fs_ops::copy_into(path, dir) {
                Ok(copy) => {
                    self.expand_to(&copy);
                    self.selected = Some(copy);
                }
                Err(problem) => problems.push(problem),
            }
        }
        self.children.clear();
        self.rebuild();
        self.reveal_selected();
        if !problems.is_empty() {
            cx.emit(FileTreeEvent::Notice(problems.join("; ")));
        }
        cx.notify();
    }

    /// Renames what the workspace was asked about (see [`FileTreeEvent::RenameRequested`]).
    pub fn finish_rename(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        // A new name in the same folder, or (dragged) the same name in another.
        let to = fs_ops::move_path(from, to)?;
        self.children.clear();
        self.expand_to(&to);
        self.selected = Some(to.clone());
        self.rebuild();
        self.reveal_selected();
        cx.emit(FileTreeEvent::Renamed { from: from.to_path_buf(), to });
        cx.notify();
        Ok(())
    }

    fn cancel_edit(&mut self, _: &CancelEdit, window: &mut Window, cx: &mut Context<Self>) {
        self.edit = None;
        self.rebuild();
        window.focus(&self.focus_handle);
        cx.notify();
    }

    // ---------- other operations ----------

    fn duplicate(&mut self, _: &Duplicate, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else { return };
        match fs_ops::duplicate(&entry.path) {
            Ok(copy) => {
                self.children.clear();
                self.selected = Some(copy);
                self.rebuild();
                cx.notify();
            }
            Err(message) => cx.emit(FileTreeEvent::Notice(message)),
        }
    }

    pub fn trash(&mut self, _: &Trash, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else { return };
        let what = if entry.is_dir { "folder" } else { "file" };
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Move the {what} “{}” to the Trash?", entry.name),
            Some("You can restore it from the Trash."),
            &["Move to Trash", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update_in(cx, |this, window, cx| this.trash_now(&entry, window, cx)).ok();
        })
        .detach();
    }

    fn trash_now(&mut self, entry: &Entry, window: &mut Window, cx: &mut Context<Self>) {
        // Keep a place in the tree for the keyboard: the row that takes its spot.
        let ix =
            self.rows.iter().position(|r| matches!(&r.kind, RowKind::Entry(e) if e.path == entry.path)).unwrap_or(0);
        match fs_ops::move_to_trash(&entry.path) {
            Ok(()) => {
                if let Some(parent) = entry.path.parent() {
                    self.children.remove(parent);
                }
                self.rebuild();
                let next = ix.min(self.rows.len().saturating_sub(1));
                self.selected = self.entry_at(next).map(|e| e.path.clone());
                window.focus(&self.focus_handle);
                cx.emit(FileTreeEvent::Trashed(entry.path.clone()));
                cx.emit(FileTreeEvent::Notice(format!("Moved {} to the Trash", entry.name)));
                cx.notify();
            }
            Err(message) => cx.emit(FileTreeEvent::Notice(message)),
        }
    }

    fn copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        let path = self.selected_entry().map(|e| e.path).unwrap_or_else(|| self.root.clone());
        cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
    }

    fn copy_relative_path(&mut self, _: &CopyRelativePath, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else { return };
        let relative = entry.path.strip_prefix(&self.root).unwrap_or(&entry.path).display().to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(relative));
    }

    fn reveal(&mut self, _: &Reveal, _: &mut Window, cx: &mut Context<Self>) {
        let path = self.selected_entry().map(|e| e.path).unwrap_or_else(|| self.root.clone());
        cx.reveal_path(&path);
    }

    fn collapse_all(&mut self, _: &CollapseAll, _: &mut Window, cx: &mut Context<Self>) {
        self.expanded = HashSet::from([self.root.clone()]);
        self.rebuild();
        cx.notify();
    }

    // ---------- context menu ----------

    /// What git says changed: those files are tinted, their folders get a dot.
    pub fn set_git_status(&mut self, changed: Vec<(PathBuf, crate::git::FileStatus)>, cx: &mut Context<Self>) {
        let git: HashMap<_, _> = changed.into_iter().collect();
        if git == self.git {
            return;
        }
        self.git_folders = git
            .keys()
            .flat_map(|p| p.ancestors().skip(1).take_while(|a| a.starts_with(&self.root) && *a != self.root))
            .map(Path::to_path_buf)
            .collect();
        self.git = git;
        cx.notify();
    }

    /// The files with errors in them: their names in the error colour, their folders with a
    /// red dot.
    pub fn set_errors(&mut self, files: HashSet<PathBuf>, cx: &mut Context<Self>) {
        if files == self.errors {
            return;
        }
        self.error_folders = files
            .iter()
            .flat_map(|p| p.ancestors().skip(1).take_while(|a| a.starts_with(&self.root) && *a != self.root))
            .map(Path::to_path_buf)
            .collect();
        self.errors = files;
        cx.notify();
    }

    /// Whether `path` is marked for errors: in it (a file) or under it (a folder).
    #[cfg(test)]
    pub fn marked_for_errors(&self, path: &Path) -> bool {
        self.errors.contains(path) || self.error_folders.contains(path)
    }

    fn open_menu(
        &mut self,
        target: Option<Entry>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use MenuItem::*;
        let items = match &target {
            Some(e) if e.is_dir => {
                vec![
                    NewFile,
                    NewFolder,
                    Rename,
                    Duplicate,
                    CopyPath,
                    CopyRelativePath,
                    Reveal,
                    OpenInTerminal,
                    FindInFolder,
                    Trash,
                ]
            }
            Some(e) => {
                let mut items = vec![Open];
                if opens_in_browser(&e.path) {
                    items.push(OpenInBrowser);
                }
                items.extend([
                    NewFile,
                    NewFolder,
                    Rename,
                    Duplicate,
                    CopyPath,
                    CopyRelativePath,
                    Reveal,
                    OpenInTerminal,
                ]);
                // A changed file (not one in a merge conflict: that's resolved, not discarded).
                if self.git.get(&e.path).is_some_and(|s| *s != crate::git::FileStatus::Conflicted) {
                    items.push(DiscardChanges);
                }
                items.push(Trash);
                items
            }
            None => vec![NewFile, NewFolder, CopyPath, Reveal, OpenInTerminal, CollapseAll],
        };
        self.selected = target.as_ref().map(|e| e.path.clone());
        self.menu = Some(Menu { target, position, items, selected: None });
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn run_menu_item(&mut self, item: MenuItem, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.menu.take().and_then(|m| m.target);
        self.selected = target.as_ref().map(|e| e.path.clone());
        match item {
            MenuItem::Open => {
                if let Some(entry) = target {
                    cx.emit(FileTreeEvent::Open(entry.path));
                }
            }
            MenuItem::OpenInBrowser => {
                if let Some(entry) = target {
                    open_in_browser(&entry.path, cx);
                }
            }
            MenuItem::NewFile => {
                let dir = self.dir_for(target.as_ref());
                self.start_edit(EditKind::NewFile { dir }, "", window, cx);
            }
            MenuItem::NewFolder => {
                let dir = self.dir_for(target.as_ref());
                self.start_edit(EditKind::NewFolder { dir }, "", window, cx);
            }
            MenuItem::Rename => self.rename(&Rename, window, cx),
            MenuItem::Duplicate => self.duplicate(&Duplicate, window, cx),
            MenuItem::CopyPath => self.copy_path(&CopyPath, window, cx),
            MenuItem::CopyRelativePath => self.copy_relative_path(&CopyRelativePath, window, cx),
            MenuItem::Reveal => self.reveal(&Reveal, window, cx),
            MenuItem::DiscardChanges => {
                if let Some(entry) = target
                    && let Some(&status) = self.git.get(&entry.path)
                {
                    cx.emit(FileTreeEvent::DiscardChanges(entry.path, status));
                }
            }
            MenuItem::FindInFolder => {
                if let Some(entry) = target {
                    cx.emit(FileTreeEvent::FindInFolder(entry.path));
                }
            }
            MenuItem::OpenInTerminal => {
                let dir = terminal_dir(target.as_ref().map(|e| (e.path.as_path(), e.is_dir)), &self.root);
                cx.emit(FileTreeEvent::OpenTerminal(dir));
            }
            MenuItem::CollapseAll => self.collapse_all(&CollapseAll, window, cx),
            MenuItem::Trash => self.trash(&Trash, window, cx),
        }
        cx.notify();
    }

    fn menu_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.menu {
            let len = menu.items.len() as isize;
            let next =
                menu.selected.map_or(if delta > 0 { 0 } else { len - 1 }, |s| (s as isize + delta).rem_euclid(len));
            menu.selected = Some(next as usize);
            cx.notify();
        }
    }

    fn menu_next(&mut self, _: &MenuNext, _: &mut Window, cx: &mut Context<Self>) {
        self.menu_step(1, cx);
    }

    fn menu_previous(&mut self, _: &MenuPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.menu_step(-1, cx);
    }

    fn menu_confirm(&mut self, _: &MenuConfirm, window: &mut Window, cx: &mut Context<Self>) {
        let item = self.menu.as_ref().and_then(|m| m.items.get(m.selected?).copied());
        if let Some(item) = item {
            self.run_menu_item(item, window, cx);
        }
    }

    fn menu_close(&mut self, _: &MenuClose, _: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        cx.notify();
    }

    // ---------- drawing ----------

    fn render_row(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.global::<Theme>();
        let row = &self.rows[ix];
        // Rows sit inset from the sidebar's edges, so their highlight has rounded ends.
        let indent = px(10. + row.depth as f32 * INDENT);
        let marker = |is_dir: bool, turn: f32, color: gpui::Hsla, dot: gpui::Hsla| {
            if is_dir {
                div().size(px(14.)).flex().items_center().justify_center().child(
                    svg()
                        .path(CHEVRON)
                        .size(px(12.))
                        .text_color(color)
                        .with_transformation(Transformation::rotate(radians(turn * std::f32::consts::FRAC_PI_2))),
                )
            } else {
                div()
                    .size(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(5.)).rounded(px(2.)).bg(dot))
            }
        };
        // The error (like "already exists") sits at the end of the field, in red.
        // Rows in the list don't stretch, so the field gets an explicit width:
        // the sidebar minus the indent, the marker and the padding.
        let field_width = (TREE_WIDTH - 12. - 10. - row.depth as f32 * INDENT - 14. - 8. - 12.).max(80.);
        let name_field = |edit: &Edit| {
            div()
                .w(px(field_width))
                .flex_none()
                .h(px(22.))
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(ui::R_KEY))
                .bg(theme.background)
                .border_1()
                .border_color(if edit.error.is_some() { theme.error } else { theme.caret })
                .line_height(px(18.))
                .child(div().flex_1().min_w(px(40.)).overflow_hidden().child(edit.input.clone()))
                .children(edit.error.clone().map(|e| {
                    div().flex_none().text_size(px(ui::T_XS)).text_color(theme.error).whitespace_nowrap().child(e)
                }))
        };

        let editing_this = |path: &Path| matches!(self.edit.as_ref().map(|e| &e.kind), Some(EditKind::Rename { path: p }) if p == path);
        let base = div()
            .id(ix)
            .h(px(ROW_HEIGHT))
            .mx(px(6.))
            .rounded(px(ui::R_ROW))
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(indent)
            .pr(px(6.))
            .text_size(px(ui::T_MD));
        match &row.kind {
            RowKind::NewItem { is_dir } => {
                let edit = self.edit.as_ref().expect("a new-item row exists only while editing");
                base.key_context("TreeEdit")
                    .child(marker(*is_dir, 0., theme.muted, theme.faint))
                    .child(name_field(edit))
                    .into_any_element()
            }
            RowKind::Entry(entry) => {
                let active = self.active.as_ref() == Some(&entry.path);
                let selected =
                    self.selected.as_ref() == Some(&entry.path) && self.focus_handle.contains_focused(window, cx);
                let (turn, turning) = self.chevron_turn(&entry.path, row.expanded);
                if turning {
                    window.request_animation_frame();
                }
                // Changed since the last commit: the name takes git's colour for it.
                let changed = self.git.get(&entry.path).map(|status| match status {
                    crate::git::FileStatus::Added => theme.git_added,
                    crate::git::FileStatus::Conflicted => theme.error,
                    _ => theme.git_modified,
                });
                // An error in it outweighs what git says.
                let changed = if self.errors.contains(&entry.path) { Some(theme.error) } else { changed };
                let color = if let Some(changed) = changed {
                    changed
                } else if active || selected {
                    theme.foreground
                } else if entry.ignored {
                    theme.faint
                } else {
                    theme.muted
                };
                let holds_changes = entry.is_dir && self.git_folders.contains(&entry.path);
                let holds_errors = entry.is_dir && self.error_folders.contains(&entry.path);
                let dot = if active { theme.caret } else { theme.faint };
                let path = entry.path.clone();
                let menu_entry = entry.clone();
                // Dropped on a folder: into it; on a file: beside it.
                let drop_dir = if entry.is_dir {
                    entry.path.clone()
                } else {
                    entry.path.parent().map(Path::to_path_buf).unwrap_or_default()
                };
                let dragged = DraggedEntry { path: entry.path.clone(), name: entry.name.clone() };
                let tint = theme.accent_soft;
                let row_el = base
                    // Tests find a row by its path (no cost outside them).
                    .debug_selector(|| format!("tree-row {}", entry.path.display()))
                    .text_color(color)
                    .when(active, |r| r.bg(theme.accent_soft))
                    .when(selected && !active, |r| r.bg(theme.hairline))
                    .when(!active && !selected, |r| r.hover(|s| s.bg(theme.hairline.opacity(0.6))))
                    .child(marker(entry.is_dir, turn, if active { theme.foreground } else { theme.muted }, dot));
                if editing_this(&path) {
                    let edit = self.edit.as_ref().unwrap();
                    row_el.key_context("TreeEdit").child(name_field(edit)).into_any_element()
                } else {
                    row_el
                        .child(div().flex_1().min_w_0().truncate().child(entry.name.clone()))
                        // A folder with changes inside: a small dot at the end.
                        .when(holds_changes || holds_errors, |r| {
                            let dot = if holds_errors { theme.error } else { theme.git_modified };
                            r.child(div().flex_none().size(px(5.)).rounded_full().bg(dot.opacity(0.8)))
                        })
                        .active(|s| s.opacity(0.7))
                        .on_drag(dragged, |d, _, _, cx| cx.new(|_| EntryGhost { name: d.name.clone() }))
                        .drag_over::<DraggedEntry>(move |style, _, _, _| style.bg(tint))
                        .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| style.bg(tint))
                        .on_drop(cx.listener({
                            let drop_dir = drop_dir.clone();
                            move |this, dragged: &DraggedEntry, _, cx| {
                                cx.stop_propagation();
                                this.drop_into(dragged, drop_dir.clone(), cx);
                            }
                        }))
                        .on_drop(cx.listener(move |this, dropped: &gpui::ExternalPaths, _, cx| {
                            cx.stop_propagation();
                            this.copy_in(dropped.paths(), &drop_dir, cx);
                        }))
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            this.click(ix, event.click_count(), window, cx)
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_menu(Some(menu_entry.clone()), event.position, window, cx)
                            }),
                        )
                        .into_any_element()
                }
            }
        }
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let menu = self.menu.as_ref()?;
        let theme = cx.global::<Theme>();
        let items = menu.items.iter().enumerate().map(|(i, &item)| {
            let selected = menu.selected == Some(i);
            let destructive = item == MenuItem::Trash;
            div()
                .when(item.starts_group() && i > 0, |d| {
                    d.mt(px(4.)).pt(px(4.)).border_t_1().border_color(theme.hairline)
                })
                .child(
                    div()
                        .id(("tree-menu", i))
                        .h(px(26.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .rounded(px(ui::R_ROW))
                        .cursor_pointer()
                        .text_color(if destructive { theme.error } else { theme.foreground })
                        .when(selected, |d| d.bg(theme.accent_soft))
                        .hover(|s| s.bg(theme.accent_soft))
                        .child(div().flex_1().child(item.label()))
                        .children(item.keys().map(|k| div().text_size(px(ui::T_SM)).text_color(theme.muted).child(k)))
                        .active(|s| s.opacity(0.7))
                        .on_click(
                            cx.listener(move |this, _: &ClickEvent, window, cx| this.run_menu_item(item, window, cx)),
                        ),
                )
        });
        Some(
            deferred(
                anchored().position(menu.position).snap_to_window_with_margin(px(8.)).child(
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
                            this.menu = None;
                            cx.notify();
                        }))
                        .children(items),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

impl Focusable for FileTree {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FileTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let menu = self.render_menu(cx);
        let theme = cx.global::<Theme>();
        let title = self.root.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default();
        let header_button = |id: &'static str, icon: &'static str| {
            let (label, action): (&'static str, Box<dyn gpui::Action>) = match id {
                "tree-new-file" => ("New file", Box::new(NewFile)),
                "tree-new-folder" => ("New folder", Box::new(NewFolder)),
                _ => ("Collapse all folders", Box::new(CollapseAll)),
            };
            div()
                .id(id)
                .tooltip(crate::ui::tip(label, Some(action)))
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(ui::R_KEY))
                .cursor_pointer()
                .group(id)
                .hover(|s| s.bg(theme.hairline))
                .child(
                    svg()
                        .path(icon)
                        .size(px(14.))
                        .text_color(theme.muted)
                        .group_hover(id, |s| s.text_color(theme.foreground)),
                )
        };
        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("FileTree");
        if self.menu.is_some() {
            key_context.add("menu_open");
        }
        div()
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::type_to_select))
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::new_folder))
            .on_action(cx.listener(Self::rename))
            .on_action(cx.listener(Self::duplicate))
            .on_action(cx.listener(Self::trash))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::copy_relative_path))
            .on_action(cx.listener(Self::reveal))
            .on_action(cx.listener(Self::collapse_all))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::expand_or_open))
            .on_action(cx.listener(Self::collapse))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::commit_edit))
            .on_action(cx.listener(Self::cancel_edit))
            .on_action(cx.listener(Self::menu_next))
            .on_action(cx.listener(Self::menu_previous))
            .on_action(cx.listener(Self::menu_confirm))
            .on_action(cx.listener(Self::menu_close))
            .size_full()
            .flex()
            .flex_col()
            .pt(px(10.))
            .child(
                div()
                    .group("tree-header")
                    .h(px(24.))
                    .pl(px(16.))
                    .pr(px(10.))
                    .mb(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(ui::section_heading(&title, theme).flex_1().min_w_0().truncate())
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .invisible()
                            .group_hover("tree-header", |s| s.visible())
                            .child(
                                header_button("tree-new-file", "icons/file-plus.svg")
                                    .active(|s| s.opacity(0.7))
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.selected = None;
                                        this.new_file(&NewFile, window, cx)
                                    })),
                            )
                            .child(
                                header_button("tree-new-folder", "icons/folder-plus.svg")
                                    .active(|s| s.opacity(0.7))
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.selected = None;
                                        this.new_folder(&NewFolder, window, cx)
                                    })),
                            )
                            .child(
                                header_button("tree-collapse", "icons/collapse.svg")
                                    .active(|s| s.opacity(0.7))
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.collapse_all(&CollapseAll, window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id("tree-body")
                    .flex_1()
                    .min_h_0()
                    // Dropped below the files: to the project's top folder.
                    .on_drop(cx.listener(|this, dragged: &DraggedEntry, _, cx| {
                        let root = this.root.clone();
                        this.drop_into(dragged, root, cx);
                    }))
                    .on_drop(cx.listener(|this, dropped: &gpui::ExternalPaths, _, cx| {
                        cx.stop_propagation();
                        let root = this.root.clone();
                        this.copy_in(dropped.paths(), &root, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.open_menu(None, event.position, window, cx)
                        }),
                    )
                    .child(
                        uniform_list(
                            "file-tree",
                            self.rows.len(),
                            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                                range.map(|ix| this.render_row(ix, window, cx)).collect()
                            }),
                        )
                        .track_scroll(self.scroll.clone())
                        .size_full(),
                    ),
            )
            .children(menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Typing with the files focused goes to a name starting with what's typed; the same
    /// letter again, to the next one with it.
    #[gpui::test]
    fn typing_goes_to_a_name(cx: &mut gpui::TestAppContext) {
        let dir = crate::tools::test_dir("tree-typing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        for f in ["alpha.rs", "beta.md", "Build.txt"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            bind_keys(cx);
        });
        let root = dir.clone();
        let (tree, cx) = cx.add_window_view(|_, cx| FileTree::new(root, cx));
        tree.update_in(cx, |t, window, _| window.focus(&t.focus_handle));
        let selected = |cx: &mut gpui::VisualTestContext| {
            tree.read_with(cx, |t, _| {
                t.selected.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned())
            })
        };
        cx.simulate_keystrokes("b e");
        assert_eq!(selected(cx).as_deref(), Some("beta.md"));
        // Typing on after a pause: a new name.
        tree.update(cx, |t, _| t.typed.1 -= TYPING_PAUSE * 2);
        cx.simulate_keystrokes("a");
        assert_eq!(selected(cx).as_deref(), Some("alpha.rs"));
        tree.update(cx, |t, _| t.typed.1 -= TYPING_PAUSE * 2);
        cx.simulate_keystrokes("b b");
        let second = selected(cx);
        cx.simulate_keystrokes("b");
        assert_ne!(selected(cx), second, "the same letter goes on to the next");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Which app is the browser, asked of macOS (run by hand: nothing is opened).
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn finds_the_default_browser() {
        let browser = default_browser().expect("a browser");
        println!("browser: {}", browser.display());
        assert!(browser.extension().is_some_and(|e| e == "app"));
    }

    #[test]
    fn pages_and_pictures_open_in_the_browser() {
        assert!(opens_in_browser(Path::new("site/index.HTML")));
        assert!(opens_in_browser(Path::new("logo.svg")));
        assert!(!opens_in_browser(Path::new("main.rs")));
        assert!(!opens_in_browser(Path::new("notes.md")));
    }

    #[test]
    fn a_terminal_opens_in_the_folder_chosen() {
        let root = Path::new("/p");
        assert_eq!(terminal_dir(Some((Path::new("/p/src"), true)), root), Path::new("/p/src"));
        assert_eq!(terminal_dir(Some((Path::new("/p/src/a.rs"), false)), root), Path::new("/p/src"));
        assert_eq!(terminal_dir(None, root), root);
    }

    #[test]
    fn lists_folders_first_and_dims_ignored_files() {
        let dir = crate::tools::test_dir("tree-test");
        let _ = std::fs::remove_dir_all(&dir);
        for sub in [".git", "src", "target"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join(".gitignore"), "target/\n.env\n").unwrap();
        std::fs::write(dir.join(".env"), "").unwrap();
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();

        let entries = FileTree::read_dir(&dir, false);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_ref()).collect();
        assert_eq!(names, ["src", "target", ".env", ".gitignore", "Cargo.toml"]);
        let ignored = |name: &str| entries.iter().find(|e| e.name.as_ref() == name).unwrap().ignored;
        assert!(ignored("target") && ignored(".env"));
        assert!(!ignored("src") && !ignored("Cargo.toml"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
