use crate::theme::Theme;
use gpui::{ClickEvent, Context, EventEmitter, SharedString, Window, div, prelude::*, px, uniform_list};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const ROW_HEIGHT: f32 = 26.;
const INDENT: f32 = 14.;
/// Never worth showing in a project tree.
const ALWAYS_HIDDEN: &[&str] = &[".git", ".DS_Store"];

pub enum FileTreeEvent {
    Open(PathBuf),
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

struct Row {
    entry: Entry,
    depth: usize,
    expanded: bool,
}

/// The project's files, folders first. Folders are read when first expanded.
pub struct FileTree {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
    rows: Vec<Row>,
    active: Option<PathBuf>,
}

impl EventEmitter<FileTreeEvent> for FileTree {}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let mut tree = Self {
            expanded: HashSet::from([root.clone()]),
            root,
            children: HashMap::new(),
            rows: Vec::new(),
            active: None,
        };
        tree.rebuild();
        tree
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        *self = Self::new(root);
        cx.notify();
    }

    /// Highlights `path` and expands its folders so it's visible.
    pub fn set_active(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if let Some(path) = &path
            && let Ok(relative) = path.strip_prefix(&self.root)
        {
            let mut dir = self.root.clone();
            for part in relative.parent().into_iter().flat_map(Path::components) {
                dir.push(part);
                self.expanded.insert(dir.clone());
            }
        }
        self.active = path;
        self.rebuild();
        cx.notify();
    }

    fn read_dir(dir: &Path) -> Vec<Entry> {
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
                Entry {
                    name: e.file_name().to_string_lossy().into_owned().into(),
                    is_dir: path.is_dir(),
                    ignored: !visible.contains(&path),
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
        self.push_children(&root, 0);
    }

    fn push_children(&mut self, dir: &Path, depth: usize) {
        let entries = self.children.entry(dir.to_path_buf()).or_insert_with(|| Self::read_dir(dir)).clone();
        for entry in entries {
            let expanded = entry.is_dir && self.expanded.contains(&entry.path);
            let path = entry.path.clone();
            self.rows.push(Row { entry, depth, expanded });
            if expanded {
                self.push_children(&path, depth + 1);
            }
        }
    }

    fn click(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix) else { return };
        let path = row.entry.path.clone();
        if row.entry.is_dir {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path);
            }
            self.rebuild();
            cx.notify();
        } else {
            cx.emit(FileTreeEvent::Open(path));
        }
    }
}

impl Render for FileTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let title = self.root.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default();
        div()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(12.))
            .child(
                div()
                    .px(px(16.))
                    .pb(px(8.))
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.faint)
                    .child(title),
            )
            .child(
                uniform_list(
                    "file-tree",
                    self.rows.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let theme = cx.global::<Theme>();
                        range
                            .map(|ix| {
                                let row = &this.rows[ix];
                                let active = this.active.as_ref() == Some(&row.entry.path);
                                let color = if active {
                                    theme.foreground
                                } else if row.entry.ignored {
                                    theme.faint
                                } else {
                                    theme.muted
                                };
                                let marker = if row.entry.is_dir {
                                    div().w(px(10.)).text_size(px(10.)).text_color(theme.faint).child(if row.expanded {
                                        "▾"
                                    } else {
                                        "▸"
                                    })
                                } else {
                                    div().w(px(10.)).flex().justify_center().child(
                                        div().size(px(6.)).rounded(px(2.)).bg(if active {
                                            theme.caret
                                        } else {
                                            theme.faint
                                        }),
                                    )
                                };
                                div()
                                    .id(ix)
                                    .h(px(ROW_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .pl(px(16. + row.depth as f32 * INDENT))
                                    .pr(px(12.))
                                    .text_size(px(13.))
                                    .text_color(color)
                                    .when(active, |row| row.bg(theme.accent_soft))
                                    .when(!active, |row| row.hover(|s| s.bg(theme.hairline)))
                                    .child(marker)
                                    .child(div().overflow_hidden().whitespace_nowrap().child(row.entry.name.clone()))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.click(ix, cx)))
                            })
                            .collect()
                    }),
                )
                .flex_1(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_first_and_dims_ignored_files() {
        let dir = std::env::temp_dir().join(format!("null-tree-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in [".git", "src", "target"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join(".gitignore"), "target/\n.env\n").unwrap();
        std::fs::write(dir.join(".env"), "").unwrap();
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();

        let entries = FileTree::read_dir(&dir);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_ref()).collect();
        assert_eq!(names, ["src", "target", ".env", ".gitignore", "Cargo.toml"]);
        let ignored = |name: &str| entries.iter().find(|e| e.name.as_ref() == name).unwrap().ignored;
        assert!(ignored("target") && ignored(".env"));
        assert!(!ignored("src") && !ignored("Cargo.toml"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
