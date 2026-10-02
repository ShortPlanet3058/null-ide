//! What's defined across the project: the signature lines of functions, types and
//! constants in every source file, found with plain patterns (no language server
//! needed). Suggestions use it to know names from other files.

use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Large projects are read up to this many files, and this much of each.
const MAX_FILES: usize = 3_000;
const MAX_FILE_BYTES: u64 = 400_000;
/// The outline handed to a suggestion is cut to about this size.
pub const OUTLINE_CHARS: usize = 6_000;

/// What suggestions know of the project beyond the open file, kept up to date by the
/// workspace: every definition, and the other files open in tabs.
#[derive(Default)]
pub struct ProjectContext {
    pub root: PathBuf,
    pub definitions: std::sync::Arc<Vec<Definition>>,
    /// The most recently used other tabs, with the start of their text.
    pub open_files: Vec<(PathBuf, String)>,
}

impl gpui::Global for ProjectContext {}

/// One definition: the file it's in and its signature line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub path: PathBuf,
    pub line: String,
}

/// Signature lines for the common languages, matched at the start of a line.
static DEFINITION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)^\s*(?:
            (?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:fn|struct|enum|trait|type|const|static|mod|union)\s+\w   # Rust
          | (?:async\s+)?def\s+\w | class\s+\w                                                                    # Python
          | (?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*\w                                         # JS/TS
          | (?:export\s+)?(?:abstract\s+)?(?:class|interface|type|enum)\s+\w                                       # JS/TS
          | export\s+(?:const|let)\s+\w                                                                           # JS/TS
          | func\s+(?:\([^)]*\)\s*)?\w | type\s+\w+\s+(?:struct|interface)                                       # Go
          | \#\s*define\s+\w | typedef\s | (?:struct|enum|union)\s+\w+\s*\{                                       # C
          | (?:static\s+|inline\s+|extern\s+|const\s+|unsigned\s+|signed\s+)*[A-Za-z_][\w]*[\s\*]+\**\w+\s*\([^;{}]*\)\s*[;{]?\s*$  # C functions
        )",
    )
    .unwrap()
});

const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "jsx", "ts", "tsx", "go", "c", "h", "cc", "cpp", "hpp", "hh", "java", "kt", "swift", "rb", "php",
    "cs", "lua", "zig",
];

/// Words that start a C-looking line without it being a function.
const NOT_FUNCTIONS: &[&str] = &["return", "if", "while", "for", "switch", "else", "do", "sizeof", "case"];

/// The definitions in one file's text.
pub fn definitions_in(path: &Path, text: &str) -> Vec<Definition> {
    text.lines()
        .filter(|l| l.len() < 220 && DEFINITION.is_match(l))
        .filter(|l| {
            let first = l.split_whitespace().next().unwrap_or("");
            !NOT_FUNCTIONS.contains(&first.trim_start_matches('*'))
        })
        .map(|l| Definition { path: path.to_path_buf(), line: l.trim().trim_end_matches('{').trim_end().to_string() })
        .collect()
}

pub fn is_source(path: &Path) -> bool {
    path.extension().and_then(|x| x.to_str()).is_some_and(|x| SOURCE_EXTENSIONS.contains(&x))
}

/// Every definition under `root`, respecting `.gitignore`. Slow on big projects: run it
/// off the main thread.
pub fn index(root: &Path) -> Vec<Definition> {
    ignore::WalkBuilder::new(root)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| SOURCE_EXTENSIONS.contains(&x)))
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_FILE_BYTES))
        .take(MAX_FILES)
        .filter_map(|e| Some((e.path().to_path_buf(), std::fs::read_to_string(e.path()).ok()?)))
        .flat_map(|(path, text)| definitions_in(&path, &text))
        .collect()
}

/// The definitions worth showing for a file being edited, as text: files nearest to it
/// first (same folder, then the folders around it), each with its signatures, until the
/// size limit. The file itself is left out: its own text is sent anyway.
pub fn outline_for(definitions: &[Definition], current: &Path, root: &Path, comment: &str) -> String {
    let distance = |p: &Path| {
        let a: Vec<_> = current.parent().map(|d| d.components().collect()).unwrap_or_default();
        let b: Vec<_> = p.parent().map(|d| d.components().collect()).unwrap_or_default();
        let shared = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        (a.len() - shared) + (b.len() - shared)
    };
    let mut files: Vec<&Path> = definitions.iter().map(|d| d.path.as_path()).filter(|p| *p != current).collect();
    files.dedup();
    files.sort_by_key(|p| (distance(p), p.to_path_buf()));
    files.dedup();
    let mut out = String::new();
    for file in files {
        let name = file.strip_prefix(root).unwrap_or(file).display();
        let mut section = format!("{comment} {name}\n");
        for d in definitions.iter().filter(|d| d.path == file) {
            section.push_str(&format!("{comment}   {}\n", d.line));
        }
        if out.len() + section.len() > OUTLINE_CHARS {
            break;
        }
        out.push_str(&section);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_definitions_in_common_languages() {
        let lines = |path: &str, text: &str| -> Vec<String> {
            definitions_in(Path::new(path), text).into_iter().map(|d| d.line).collect()
        };
        assert_eq!(
            lines("a.rs", "pub fn parse(s: &str) -> Ast {\n    let x = 1;\n}\nstruct Ast;\n"),
            ["pub fn parse(s: &str) -> Ast", "struct Ast;"]
        );
        assert_eq!(lines("a.py", "def load(path):\n    return 1\nclass Store:\n"), ["def load(path):", "class Store:"]);
        assert_eq!(
            lines(
                "a.c",
                "void my_putchar(char c)\n{\n    write(1, &c, 1);\n    return;\n}\nint my_strlen(char const *str);\n"
            ),
            ["void my_putchar(char c)", "int my_strlen(char const *str);"]
        );
        assert_eq!(
            lines("a.ts", "export function run(x: number) {\n  if (x) {}\n}\n"),
            ["export function run(x: number)"]
        );
    }

    #[test]
    fn the_outline_starts_with_the_nearest_files() {
        let d = |p: &str, l: &str| Definition { path: PathBuf::from(p), line: l.into() };
        let defs =
            [d("/p/far/x.c", "int far(void)"), d("/p/src/near.c", "int near(void)"), d("/p/src/me.c", "int me(void)")];
        let outline = outline_for(&defs, Path::new("/p/src/me.c"), Path::new("/p"), "//");
        assert!(outline.find("near").unwrap() < outline.find("far").unwrap());
        assert!(!outline.contains("me(void)"));
    }
}
