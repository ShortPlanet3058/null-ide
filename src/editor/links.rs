//! ⌘-click on a web address or a file's path written in the text (a comment, a string, a
//! README): the address opens in the browser, the file in Null, at its line if one follows
//! (`src/a.rs:12:5`). Anywhere else, ⌘-click goes to the definition.

use super::{Editor, EditorEvent};
use crate::terminal_links::{Link, link_at};
use gpui::Context;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Lines longer than this aren't looked through (a minified file).
const LONGEST: usize = 4000;
/// How many folders up from the file a relative path is looked for (the project's root is
/// one of them).
const FOLDERS_UP: usize = 8;

pub(super) enum Target {
    Url(String),
    File { path: PathBuf, line: Option<u32>, column: Option<u32> },
}

/// Where `written` (as in the text) is, from the file in `dir`: there, or in a folder above.
fn resolve(written: &str, dir: Option<&Path>) -> Option<PathBuf> {
    // A Markdown link's `#heading` isn't part of the file's name.
    let written = written.split('#').next().filter(|w| !w.is_empty())?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = match (written.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(written),
    };
    if path.is_absolute() {
        return path.is_file().then_some(path);
    }
    dir?.ancestors().take(FOLDERS_UP).map(|d| d.join(&path)).find(|p| p.is_file())
}

impl Editor {
    /// The address or existing file written at `offset`, and the chars it covers.
    pub(super) fn link_under(&self, offset: usize) -> Option<(Range<usize>, Target)> {
        let (line, column) = self.buffer.point(offset);
        if self.buffer.line_len(line) > LONGEST {
            return None;
        }
        let row: Vec<char> = self.buffer.line_text(line).chars().collect();
        // A click lands on the nearest gap between letters: past a link's last letter too.
        let (columns, link) = link_at(&row, column).or_else(|| link_at(&row, column.checked_sub(1)?))?;
        let start = self.buffer.line_to_char(line);
        let target = match link {
            Link::Url(url) => Target::Url(url),
            Link::File { path, line, column } => {
                let dir = self.path.as_deref().and_then(Path::parent);
                Target::File { path: resolve(&path, dir)?, line, column }
            }
        };
        Some((start + columns.start..start + columns.end, target))
    }

    /// Opens what ⌘-click found: the address in the browser, the file in Null.
    pub(super) fn follow_link(&mut self, target: Target, cx: &mut Context<Self>) {
        match target {
            Target::Url(url) => cx.open_url(&url),
            Target::File { path, line, column } => {
                let at = lsp_types::Position {
                    line: line.unwrap_or(1).saturating_sub(1),
                    character: column.unwrap_or(1).saturating_sub(1),
                };
                cx.emit(EditorEvent::GoTo { path, range: lsp_types::Range { start: at, end: at } });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn finds_addresses_and_files_that_exist(cx: &mut TestAppContext) {
        let root = crate::tools::test_dir("editor-links");
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(root.join("README.md"), "").unwrap();
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let text = "// See https://example.com/docs, src/lib.rs:12:5 and [the readme](README.md#setup).\n\
                    let x = self.buffer.slice(a);\n";
        let path = root.join("src/deep/x.rs");
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(crate::buffer::Buffer::from_text(text), Some(path), cx));
        e.update(cx, |e, _| {
            let at = |needle: &str| text.find(needle).unwrap() + 2;
            let (range, target) = e.link_under(at("https")).unwrap();
            assert_eq!(e.buffer.slice(range), "https://example.com/docs");
            assert!(matches!(target, Target::Url(u) if u == "https://example.com/docs"));
            // Found from the project's root, two folders up.
            let (range, target) = e.link_under(at("src/lib")).unwrap();
            assert_eq!(e.buffer.slice(range), "src/lib.rs:12:5");
            assert!(matches!(target, Target::File { path, line: Some(12), column: Some(5) } if path == root.join("src/lib.rs")));
            let (_, target) = e.link_under(at("README")).unwrap();
            assert!(matches!(target, Target::File { path, line: None, .. } if path == root.join("README.md")));
            // Just past the address's last letter (where a click on its right half lands).
            let end = text.find("/docs").unwrap() + 5;
            assert!(matches!(e.link_under(end), Some((_, Target::Url(_)))));
            // Code that only looks like a file name is no link.
            assert!(e.link_under(at("self.buffer")).is_none());
        });
        std::fs::remove_dir_all(&root).ok();
    }
}
