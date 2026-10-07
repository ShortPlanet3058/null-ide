//! Broken links in Markdown: a link or image to a file that isn't there, or to a
//! `#section` no heading of the file makes, gets a wavy line; ⌘. on it offers the names
//! nearest to what's written, from that folder (or the file's sections).

use super::Editor;
use super::fixes::FixMenu;
use gpui::{App, Context, ScrollHandle};
use lsp_types::{CodeActionOrCommand, Command};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Whether a file exists is asked again after this long (it may have been made since).
const KNOWN_FOR: Duration = Duration::from_secs(2);
/// At most this many names are offered.
const CHOICES: usize = 5;

/// The `#anchors` a Markdown text's headings make (`## Get started` → `get-started`).
fn anchors(text: &str) -> Vec<String> {
    crate::markdown_view::headings(text).into_iter().map(|(_, _, anchor)| anchor).collect()
}

impl Editor {
    /// The Markdown file's folder, where its relative links start.
    fn link_folder(&self) -> Option<PathBuf> {
        self.path.as_ref()?.parent().map(Path::to_path_buf)
    }

    /// The file a link names (`%20` read as a space), from this file's folder.
    fn linked_file(&self, written: &str) -> Option<PathBuf> {
        let path = crate::markdown_links::decoded(written);
        // From the site's root (`/docs/a.md`): where that is isn't known here.
        if path.is_empty() || path.starts_with('/') {
            return None;
        }
        Some(crate::markdown_links::tidy(&self.link_folder()?.join(path)))
    }

    /// Whether a file is there, remembered a moment so drawing doesn't ask the disk each frame.
    fn exists(&self, path: &Path) -> bool {
        let mut known = self.link_targets.borrow_mut();
        if let Some((there, when)) = known.get(path)
            && when.elapsed() < KNOWN_FOR
        {
            return *there;
        }
        let there = path.exists();
        if known.len() > 1000 {
            known.clear();
        }
        known.insert(path.to_path_buf(), (there, Instant::now()));
        there
    }

    /// This file's `#anchors`, worked out once per version of its text.
    fn doc_anchors(&self) -> Vec<String> {
        self.with_anchors(|anchors| anchors.to_vec())
    }

    /// `f` given this file's anchors, without copying them (drawing asks for each `#link`).
    fn with_anchors<T>(&self, f: impl FnOnce(&[String]) -> T) -> T {
        let mut cache = self.anchors.borrow_mut();
        let version = self.buffer.version();
        if cache.as_ref().is_none_or(|(v, _)| *v != version) {
            *cache = Some((version, anchors(&self.buffer.to_string())));
        }
        f(cache.as_ref().map_or(&[][..], |(_, a)| a.as_slice()))
    }

    /// Whether the link written `written` leads nowhere.
    fn link_is_broken(&self, written: &str) -> bool {
        match (self.linked_file(written), written.strip_prefix('#')) {
            (Some(file), _) => !self.exists(&file),
            // This file's own section: one of its headings.
            // `#` alone: the top of the page.
            (None, Some(anchor)) => !anchor.is_empty() && !self.with_anchors(|a| a.iter().any(|a| a == anchor)),
            (None, None) => false,
        }
    }

    /// The broken links on line `line` (its text `text`), by byte range in it.
    pub fn broken_links_on_line(&self, line: usize, text: &str) -> Vec<Range<usize>> {
        if !self.is_markdown() || self.path.is_none() || self.in_fence(line) {
            return Vec::new();
        }
        let code = crate::markdown_links::code_spans(text);
        crate::markdown_links::targets(text)
            .into_iter()
            .chain(text.match_indices("](#").filter(|(at, _)| !code.iter().any(|c| c.contains(at))).map(|(at, _)| {
                let start = at + 2;
                let end = text[start..].find([')', ' ']).map_or(text.len(), |e| start + e);
                crate::markdown_links::Target { line: 0, start, text: &text[start..end] }
            }))
            .filter(|t| self.link_is_broken(t.text))
            .map(|t| t.start..t.start + t.text.len())
            .collect()
    }

    /// ⌘. on a broken link: the existing names nearest to it, to change it to. True when the
    /// caret was on one.
    pub(super) fn broken_link_choices(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((range, written)) = self.broken_link_at_caret(cx) else { return false };
        let choices = match written.strip_prefix('#') {
            Some(anchor) => nearest(anchor, self.doc_anchors()).into_iter().map(|a| format!("#{a}")).collect(),
            None => self.nearest_files(&written),
        };
        if choices.is_empty() {
            self.show_notice(range.start, "Nothing there by a name like that.".into(), cx);
            return true;
        }
        let fixes = choices
            .into_iter()
            .map(|choice| {
                CodeActionOrCommand::Command(Command {
                    title: choice.clone(),
                    command: super::spelling::CHANGE.into(),
                    arguments: Some(vec![serde_json::Value::String(choice)]),
                })
            })
            .collect();
        self.close_hover(cx);
        self.fix_menu = Some(FixMenu {
            fixes,
            selected: 0,
            at: range.start,
            scroll: ScrollHandle::new(),
            conflict: None,
            spelling: Some(range),
        });
        cx.notify();
        true
    }

    /// The broken link the caret is in, by char range, and as written.
    fn broken_link_at_caret(&self, cx: &App) -> Option<(Range<usize>, String)> {
        let _ = cx;
        let (line, column) = self.caret_point();
        let text = self.buffer.line_text(line);
        let at = text.char_indices().nth(column).map_or(text.len(), |(b, _)| b);
        let link = self.broken_links_on_line(line, &text).into_iter().find(|r| r.start <= at && at <= r.end)?;
        let start = self.buffer.rope().line_to_char(line) + text[..link.start].chars().count();
        let written = text[link.clone()].to_string();
        Some((start..start + written.chars().count(), written))
    }

    /// For a link to a missing file: the names nearest to it in the folder it names (or, if
    /// that folder isn't there either, in this file's), written as the link was.
    fn nearest_files(&self, written: &str) -> Vec<String> {
        let (path, fragment) = match written.find('#') {
            Some(at) => (&written[..at], &written[at..]),
            None => (written, ""),
        };
        let (folder_written, name) = match path.rfind('/') {
            Some(at) => (&path[..=at], &path[at + 1..]),
            None => ("", path),
        };
        let Some(here) = self.link_folder() else { return Vec::new() };
        let folder = crate::markdown_links::tidy(&here.join(folder_written.replace("%20", " ")));
        let (folder, prefix) = if folder.is_dir() { (folder, folder_written) } else { (here, "") };
        let Ok(entries) = std::fs::read_dir(&folder) else { return Vec::new() };
        let names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| !n.starts_with('.'))
            .collect();
        let encode = written.contains("%20");
        nearest(&name.replace("%20", " "), names)
            .into_iter()
            .map(|n| {
                let n = if encode { n.replace(' ', "%20") } else { n };
                format!("{prefix}{n}{fragment}")
            })
            .collect()
    }
}

/// The names in `names` most like `wanted`: same letters in order first, then the most in
/// common.
fn nearest(wanted: &str, names: Vec<String>) -> Vec<String> {
    let wanted = wanted.to_lowercase();
    let mut scored: Vec<(i64, String)> = names
        .into_iter()
        .map(|name| {
            let lower = name.to_lowercase();
            let fuzzy = crate::fuzzy::score(&lower, &wanted).map_or(0, |(s, _)| s as i64 + 1000);
            let shared = wanted.chars().filter(|c| lower.contains(*c)).count() as i64;
            let stem = |s: &str| s.split('.').next().unwrap_or(s).to_string();
            let same_stem = i64::from(stem(&lower) == stem(&wanted)) * 500;
            (fuzzy + shared * 10 + same_stem - (lower.len() as i64 - wanted.len() as i64).abs(), name)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().take(CHOICES).map(|(_, n)| n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_make_anchors() {
        let text =
            "# Null\n\n## Get started\n```\n# not a heading\n```\n### Keys & shortcuts ###\n#hashtag\n## Get started\n";
        assert_eq!(anchors(text), ["null", "get-started", "keys--shortcuts", "get-started-1"]);
    }

    /// A link to a file that isn't there is marked, one to a section that is isn't; ⌘. on
    /// it offers the nearest name, ↵ takes it.
    #[gpui::test]
    fn broken_links_are_marked_and_fixed(cx: &mut gpui::TestAppContext) {
        use crate::fonts::Fonts;
        use crate::settings::Settings;
        use crate::theme::Theme;
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let dir = std::env::temp_dir().join(format!("null-broken-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/setup.md"), "x").unwrap();
        let notes = dir.join("notes.md");
        std::fs::write(
            &notes,
            "See [setup](docs/setpu.md), [intro](#intro), [x](#nowhere).\n# Intro\n[top](#) `[a](#b)` [s](/docs/x.md)\n[^1]: Some note\n",
        )
        .unwrap();
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::open(notes.clone(), None, cx));
        editor.update_in(cx, |e, window, cx| {
            let text = e.buffer.line_text(0);
            let broken: Vec<String> =
                e.broken_links_on_line(0, &text).into_iter().map(|r| text[r].to_string()).collect();
            assert_eq!(broken, ["docs/setpu.md", "#nowhere"]);
            // The top of the page, a link in code, one from the site's root, a footnote: fine.
            for line in [2, 3] {
                assert!(e.broken_links_on_line(line, &e.buffer.line_text(line)).is_empty(), "line {line}");
            }
            window.focus(&e.focus_handle);
            e.selection = super::super::Selection::caret(15);
            cx.notify();
        });
        cx.simulate_keystrokes("cmd-. enter");
        editor.read_with(cx, |e, _| assert!(e.buffer.to_string().starts_with("See [setup](docs/setup.md),")));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// What a frame of a big Markdown file costs in link and spelling checks, right after
    /// a keystroke (anchors and fences worked out again). Run by hand:
    /// `cargo test timing_markdown -- --ignored --nocapture`
    #[gpui::test]
    #[ignore]
    fn timing_markdown_frame(cx: &mut gpui::TestAppContext) {
        use crate::fonts::Fonts;
        use crate::settings::Settings;
        use crate::theme::Theme;
        cx.update(|cx| {
            cx.set_global(Settings::default());
            cx.set_global(Theme::oled());
            cx.set_global(Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let dir = std::env::temp_dir().join(format!("null-timing-markdown-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut text = String::from("# Notes\n\n");
        for i in 0..4000 {
            text.push_str(&format!("Some words teh here [link](docs/x{i}.md) ![i](img/{i}.png) [s](#notes)\n"));
            if i % 20 == 0 {
                text.push_str(&format!("## Part {i}\n"));
            }
        }
        let path = dir.join("big.md");
        std::fs::write(&path, &text).unwrap();
        let (editor, cx) = cx.add_window_view(|_, cx| Editor::open(path.clone(), None, cx));
        editor.update(cx, |e, cx| {
            let frame = |e: &Editor, cx: &App| {
                for line in 2000..2060 {
                    let text = e.buffer.line_text(line);
                    e.broken_links_on_line(line, &text);
                    e.misspellings_on_line(line, &text, cx);
                }
            };
            frame(e, cx);
            let start = Instant::now();
            for i in 0..10 {
                e.edit(0..0, if i % 2 == 0 { "x" } else { "" }, super::super::EditKind::Typing, cx);
                frame(e, cx);
            }
            println!("a frame after a keystroke, 60 lines: {:?}", start.elapsed() / 10);
            let start = Instant::now();
            for _ in 0..10 {
                frame(e, cx);
            }
            println!("a frame with nothing typed: {:?}", start.elapsed() / 10);
            let start = Instant::now();
            for _ in 0..10 {
                anchors(&e.buffer.to_string());
            }
            println!("  anchors of the whole file: {:?}", start.elapsed() / 10);
            let start = Instant::now();
            for _ in 0..10 {
                super::super::spelling::fenced_lines_for_timing(e.buffer.rope());
            }
            println!("  fences of the whole file: {:?}", start.elapsed() / 10);
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nearest_names_first() {
        let names = vec!["setup.md".to_string(), "install.md".to_string(), "README.md".to_string()];
        assert_eq!(nearest("setpu.md", names.clone())[0], "setup.md");
        assert_eq!(nearest("instal.md", names)[0], "install.md");
    }
}
