//! Which language a file is in: the one chosen for it (a click on the language's name in
//! the status bar, or ⌘K "Language: …"), else the one its name says, else the one its first
//! line says (`#!/usr/bin/env python3`, `<?xml`), so scripts without an extension and
//! untitled files get their colours too.

use super::Editor;
use crate::basic_syntax::Basic;
use crate::languages::Language;
use gpui::{Context, Window};

/// A language, as Null colours it.
#[derive(Clone, Copy)]
pub enum Kind {
    Grammar(&'static Language),
    Basic(&'static Basic),
    /// No colours at all, chosen.
    Plain,
}

impl std::fmt::Debug for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl PartialEq for Kind {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Grammar(l) => l.name,
            Kind::Basic(b) => b.name,
            Kind::Plain => PLAIN,
        }
    }
}

pub const PLAIN: &str = "Plain Text";

/// Puts the file in this language (by its name; "Plain Text" for none).
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetLanguage {
    pub name: &'static str,
}

/// The language called `name`.
pub fn by_name(name: &str) -> Option<Kind> {
    if name == PLAIN {
        return Some(Kind::Plain);
    }
    crate::languages::all()
        .iter()
        .find(|l| l.name == name)
        .map(Kind::Grammar)
        .or_else(|| crate::basic_syntax::all().iter().find(|b| b.name == name).map(Kind::Basic))
}

/// Every language's name, A to Z, Plain Text last.
pub fn names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = crate::languages::all()
        .iter()
        .map(|l| l.name)
        .chain(crate::basic_syntax::all().iter().map(|b| b.name))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names.push(PLAIN);
    names
}

pub fn by_path(path: &std::path::Path) -> Option<Kind> {
    crate::languages::for_path(path).map(Kind::Grammar).or_else(|| crate::basic_syntax::for_path(path).map(Kind::Basic))
}

/// The language a file's first line says: a script's `#!` line, an XML or HTML start.
pub fn from_first_line(line: &str) -> Option<Kind> {
    let line = line.trim_start_matches('\u{feff}').trim_end();
    let name = if let Some(command) = line.strip_prefix("#!") {
        let mut words = command.split_whitespace();
        let mut program = words.next()?.rsplit('/').next()?;
        // `#!/usr/bin/env -S python3 -u`: the program is the first word that isn't an option.
        if program == "env" {
            program = words.find(|w| !w.starts_with('-') && !w.contains('='))?;
        }
        // `python3.12` is Python.
        match program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.') {
            "python" | "pypy" => "Python",
            "sh" | "bash" | "zsh" | "dash" | "ksh" | "ash" => "Shell",
            "node" | "nodejs" | "deno" | "bun" => "JavaScript",
            "ruby" => "Ruby",
            "lua" | "luajit" => "Lua",
            "php" => "PHP",
            "swift" => "Swift",
            "make" | "gmake" => "Makefile",
            _ => return None,
        }
    } else {
        let lower: String = line.chars().take(32).collect::<String>().to_ascii_lowercase();
        if lower.starts_with("<?xml") {
            "XML"
        } else if lower.starts_with("<?php") {
            "PHP"
        } else if lower.starts_with("<!doctype html") || lower.starts_with("<html") {
            "HTML"
        } else {
            return None;
        }
    };
    by_name(name)
}

impl Editor {
    /// The file's language: chosen, from its name, or from its first line.
    pub fn kind(&self) -> Option<Kind> {
        self.chosen_language.or_else(|| self.path.as_deref().and_then(by_path)).or(self.first_line_language)
    }

    /// The language chosen for this file, if one was (kept with the session).
    pub fn chosen_language(&self) -> Option<&'static str> {
        self.chosen_language.map(|k| k.name())
    }

    pub(super) fn set_language(&mut self, action: &SetLanguage, _: &mut Window, cx: &mut Context<Self>) {
        self.choose_language(by_name(action.name), cx);
    }

    /// Puts the file in `kind` (None: back to the one its name or first line says).
    pub fn choose_language(&mut self, kind: Option<Kind>, cx: &mut Context<Self>) {
        // Chosen the one it would be anyway: nothing to remember.
        let by_itself = self.path.as_deref().and_then(by_path).or(self.first_line_language);
        let kind = kind.filter(|k| Some(*k) != by_itself);
        if kind == self.chosen_language {
            return;
        }
        self.chosen_language = kind;
        self.language_changed(cx);
    }

    /// After an edit: a file whose name says nothing takes the language its first line says.
    pub(super) fn first_line_after_edit(&mut self, cx: &mut Context<Self>) {
        if self.chosen_language.is_some() || self.path.as_deref().and_then(by_path).is_some() {
            return;
        }
        let rope = self.buffer.rope();
        let first = rope.line(0);
        let head: String = first.chars().take(200).collect();
        let now = from_first_line(&head);
        if now != self.first_line_language {
            self.first_line_language = now;
            self.language_changed(cx);
        }
    }

    /// Colours again, in the language the file is in now.
    pub(super) fn language_changed(&mut self, cx: &mut Context<Self>) {
        self.highlighter = super::highlighter_for(self.language(), &self.buffer);
        self.spans.clear();
        self.spans_for = None;
        self.pinned_spans.clear();
        self.rehighlight();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_lines_tell_the_language() {
        let name = |line: &str| from_first_line(line).map(|k| k.name());
        assert_eq!(name("#!/usr/bin/env python3"), Some("Python"));
        assert_eq!(name("#!/usr/bin/python3.12 -u"), Some("Python"));
        assert_eq!(name("#!/bin/bash"), Some("Shell"));
        assert_eq!(name("#!/usr/bin/env -S node --no-warnings"), Some("JavaScript"));
        assert_eq!(name("#!/usr/bin/env ruby"), Some("Ruby"));
        assert_eq!(name("<?xml version=\"1.0\"?>"), Some("XML"));
        assert_eq!(name("<!DOCTYPE html>"), Some("HTML"));
        assert_eq!(name("<?php echo 1;"), Some("PHP"));
        assert_eq!(name("#!/usr/bin/env perl"), None);
        assert_eq!(name("# a title"), None);
        assert_eq!(name(""), None);
        assert_eq!(name("é"), None);
    }

    #[gpui::test]
    fn a_file_s_language_comes_from_its_name_first_line_or_choice(cx: &mut gpui::TestAppContext) {
        use crate::buffer::Buffer;
        use gpui::AppContext as _;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
        });
        // A script without an extension: its `#!` line.
        let script =
            cx.new(|cx| Editor::new(Buffer::from_text("#!/usr/bin/env python3\nx = 1\n"), Some("deploy".into()), cx));
        script.read_with(cx, |e, _| {
            assert_eq!(e.language_name(), "Python");
            assert!(e.highlighter.is_some(), "coloured by the grammar");
        });
        // Untitled: the language follows the first line as it's typed.
        let untitled = cx.new(|cx| Editor::new(Buffer::from_text(""), None, cx));
        untitled.update(cx, |e, cx| {
            assert_eq!(e.language_name(), PLAIN);
            e.edit(0..0, "#!/bin/bash\necho hi\n", super::super::EditKind::Other, cx);
            assert_eq!(e.language_name(), "Shell");
            // Chosen: Ruby, over the first line; Plain Text, no colours.
            e.choose_language(by_name("Ruby"), cx);
            assert_eq!((e.language_name(), e.chosen_language()), ("Ruby", Some("Ruby")));
            assert!(e.basic_syntax().is_some());
            e.choose_language(by_name(PLAIN), cx);
            assert!(e.language().is_none() && e.basic_syntax().is_none());
            assert_eq!(e.language_name(), PLAIN);
            // Chosen what it would be anyway: nothing kept.
            e.choose_language(by_name("Shell"), cx);
            assert_eq!((e.language_name(), e.chosen_language()), ("Shell", None));
        });
        // A name that says: the first line doesn't change it.
        let rust = cx.new(|cx| Editor::new(Buffer::from_text("#!/bin/bash\n"), Some("a.rs".into()), cx));
        rust.read_with(cx, |e, _| assert_eq!(e.language_name(), "Rust"));
    }

    #[test]
    fn languages_are_found_by_name() {
        assert_eq!(by_name("Rust").map(|k| k.name()), Some("Rust"));
        assert_eq!(by_name("Swift").map(|k| k.name()), Some("Swift"));
        assert_eq!(by_name(PLAIN), Some(Kind::Plain));
        assert_eq!(by_name("Cobol"), None);
        let all = names();
        assert!(all.contains(&"Python") && all.contains(&"Kotlin"));
        assert_eq!(all.last(), Some(&PLAIN));
        assert!(all.iter().all(|n| by_name(n).is_some()), "every name listed is one");
    }
}
