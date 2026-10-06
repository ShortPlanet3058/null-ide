//! The languages Null knows: how to recognise their files, their grammar and
//! highlighting rules, and how they write comments.

use crate::highlight::HighlightQuery;
use std::path::Path;
use std::sync::{LazyLock, OnceLock};

pub struct Language {
    /// As people call it: "Python", "TypeScript".
    pub name: &'static str,
    extensions: &'static [&'static str],
    /// Whole file names, for files without a telling extension.
    file_names: &'static [&'static str],
    grammar: fn() -> tree_sitter::Language,
    /// Highlight queries, concatenated: languages that extend another (TypeScript
    /// extends JavaScript, C++ extends C) list the base first.
    highlights: &'static [&'static str],
    /// How a line comment starts, for toggling comments. None when there is only a block form.
    pub line_comment: Option<&'static str>,
    /// How a block comment opens and closes, if the language has one (⌥⌘/; ⌘/ uses it only
    /// when there are no line comments).
    pub block_comment: Option<(&'static str, &'static str)>,
    query: OnceLock<Option<HighlightQuery>>,
}

macro_rules! language {
    ($name:expr, [$($ext:expr),*], [$($file:expr),*], $grammar:expr, [$($query:expr),+], $comment:expr) => {
        Language {
            name: $name,
            extensions: &[$($ext),*],
            file_names: &[$($file),*],
            grammar: || $grammar.into(),
            highlights: &[$($query),+],
            line_comment: $comment,
            block_comment: None,
            query: OnceLock::new(),
        }
    };
}

static LANGUAGES: LazyLock<Vec<Language>> = LazyLock::new(|| {
    let mut languages = vec![
        language!("Rust", ["rs"], [], tree_sitter_rust::LANGUAGE, [tree_sitter_rust::HIGHLIGHTS_QUERY], Some("//")),
        language!(
            "Python",
            ["py", "pyi", "pyw"],
            [],
            tree_sitter_python::LANGUAGE,
            [tree_sitter_python::HIGHLIGHTS_QUERY],
            Some("#")
        ),
        language!(
            "JavaScript",
            ["js", "mjs", "cjs", "jsx"],
            [],
            tree_sitter_javascript::LANGUAGE,
            [tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY],
            Some("//")
        ),
        language!(
            "TypeScript",
            ["ts", "mts", "cts"],
            [],
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            [tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_typescript::HIGHLIGHTS_QUERY],
            Some("//")
        ),
        language!(
            "TSX",
            ["tsx"],
            [],
            tree_sitter_typescript::LANGUAGE_TSX,
            [
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            ],
            Some("//")
        ),
        language!(
            "JSON",
            ["json", "jsonc", "json5"],
            [".prettierrc", ".eslintrc"],
            tree_sitter_json::LANGUAGE,
            [tree_sitter_json::HIGHLIGHTS_QUERY],
            None
        ),
        language!(
            "TOML",
            ["toml"],
            ["Cargo.lock", "Pipfile"],
            tree_sitter_toml_ng::LANGUAGE,
            [tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
            Some("#")
        ),
        language!(
            "Markdown",
            ["md", "markdown", "mdx"],
            [],
            tree_sitter_md::LANGUAGE,
            [tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
            None
        ),
        language!(
            "HTML",
            ["html", "htm", "xhtml"],
            [],
            tree_sitter_html::LANGUAGE,
            [tree_sitter_html::HIGHLIGHTS_QUERY],
            None
        ),
        language!("CSS", ["css"], [], tree_sitter_css::LANGUAGE, [tree_sitter_css::HIGHLIGHTS_QUERY], None),
        language!("Go", ["go"], [], tree_sitter_go::LANGUAGE, [tree_sitter_go::HIGHLIGHTS_QUERY], Some("//")),
        language!("C", ["c", "h"], [], tree_sitter_c::LANGUAGE, [tree_sitter_c::HIGHLIGHT_QUERY], Some("//")),
        language!(
            "C++",
            ["cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp"],
            [],
            tree_sitter_cpp::LANGUAGE,
            [tree_sitter_c::HIGHLIGHT_QUERY, tree_sitter_cpp::HIGHLIGHT_QUERY],
            Some("//")
        ),
        language!(
            "YAML",
            ["yml", "yaml"],
            [".clang-format"],
            tree_sitter_yaml::LANGUAGE,
            [tree_sitter_yaml::HIGHLIGHTS_QUERY],
            Some("#")
        ),
        language!(
            "Shell",
            ["sh", "bash", "zsh", "command"],
            [".bashrc", ".bash_profile", ".zshrc", ".zprofile", ".profile", ".zshenv"],
            tree_sitter_bash::LANGUAGE,
            [tree_sitter_bash::HIGHLIGHT_QUERY],
            Some("#")
        ),
    ];
    for language in &mut languages {
        language.block_comment = match language.name {
            "HTML" | "Markdown" => Some(("<!--", "-->")),
            "CSS" | "Rust" | "JavaScript" | "TypeScript" | "TSX" | "Go" | "C" | "C++" => Some(("/*", "*/")),
            _ => None,
        };
    }
    languages
});

/// The language of a file, from its name or extension.
pub fn for_path(path: &Path) -> Option<&'static Language> {
    let name = path.file_name()?.to_str()?;
    if let Some(language) = LANGUAGES.iter().find(|l| l.file_names.contains(&name)) {
        return Some(language);
    }
    let ext = path.extension()?.to_str()?.to_lowercase();
    LANGUAGES.iter().find(|l| l.extensions.contains(&ext.as_str()))
}

impl Language {
    pub fn grammar(&self) -> tree_sitter::Language {
        (self.grammar)()
    }

    /// Built on first use and shared by every file in this language.
    pub fn highlight_query(&'static self) -> Option<&'static HighlightQuery> {
        self.query
            .get_or_init(|| match HighlightQuery::new(&self.grammar(), &self.highlights.join("\n")) {
                Ok(query) => Some(query),
                Err(err) => {
                    eprintln!("null: highlighting for {} is unavailable: {err}", self.name);
                    None
                }
            })
            .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_files_by_extension_and_name() {
        let name = |p: &str| for_path(Path::new(p)).map(|l| l.name);
        assert_eq!(name("src/main.rs"), Some("Rust"));
        assert_eq!(name("app/models.PY"), Some("Python"));
        assert_eq!(name("web/App.tsx"), Some("TSX"));
        assert_eq!(name("Cargo.lock"), Some("TOML"));
        assert_eq!(name("/home/me/.zshrc"), Some("Shell"));
        assert_eq!(name("notes.txt"), None);
    }

    #[test]
    fn every_language_builds_its_highlighting() {
        for language in LANGUAGES.iter() {
            assert!(language.highlight_query().is_some(), "{} failed", language.name);
        }
    }
}
