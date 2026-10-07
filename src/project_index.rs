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

/// One definition: the file it's in, its signature line, where it is, and what it's called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub path: PathBuf,
    pub line: String,
    /// Zero-based line number.
    pub row: usize,
    pub name: String,
    /// The word that defines it, as the language writes it: `fn`, `class`, `struct`...
    pub kind: &'static str,
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

/// How a definition line names what it defines, most specific first: (pattern, kind).
/// A kind of "" takes the keyword the pattern matched.
static NAMES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"#\s*define\s+([A-Za-z_]\w*)", "macro"),
        (r"\btypedef\b.*?([A-Za-z_]\w*)\s*;", "type"),
        (r"\bfunc\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)", "func"),
        (r"\b(fn|def|class|function\s*\*?|struct|enum|trait|interface|type|mod|union)\s+([A-Za-z_$][\w$]*)", ""),
        (r"\b(const|static|let)\s+(?:mut\s+)?([A-Za-z_$][\w$]*)", ""),
        (r"([A-Za-z_]\w*)\s*\(", "fn"),
    ]
    .into_iter()
    .map(|(pattern, kind)| (Regex::new(pattern).unwrap(), kind))
    .collect()
});

/// The name a definition line defines, and its kind.
fn name_and_kind(line: &str) -> Option<(String, &'static str)> {
    NAMES.iter().find_map(|(regex, kind)| {
        let caps = regex.captures(line)?;
        if kind.is_empty() {
            let keyword = caps.get(1)?.as_str().split_whitespace().next()?.trim_end_matches('*');
            let keyword = [
                "fn",
                "def",
                "class",
                "function",
                "struct",
                "enum",
                "trait",
                "interface",
                "type",
                "mod",
                "union",
                "const",
                "static",
                "let",
            ]
            .into_iter()
            .find(|k| *k == keyword)?;
            Some((caps.get(2)?.as_str().to_string(), keyword))
        } else {
            Some((caps.get(1)?.as_str().to_string(), *kind))
        }
    })
}

/// The definitions in one file's text.
pub fn definitions_in(path: &Path, text: &str) -> Vec<Definition> {
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase);
    if matches!(extension.as_deref(), Some("md" | "markdown" | "mdx")) {
        return headings_in(path, text);
    }
    text.lines()
        .enumerate()
        .filter(|(_, l)| l.len() < 220 && DEFINITION.is_match(l))
        .filter(|(_, l)| {
            let first = l.split_whitespace().next().unwrap_or("");
            !NOT_FUNCTIONS.contains(&first.trim_start_matches('*'))
        })
        .filter_map(|(row, l)| {
            let line = l.trim().trim_end_matches('{').trim_end().to_string();
            let (name, kind) = name_and_kind(&line)?;
            Some(Definition { path: path.to_path_buf(), line, row, name, kind })
        })
        .collect()
}

/// A Markdown file's outline, for ⌘⇧O: its headings (not lines starting with # in code).
fn headings_in(path: &Path, text: &str) -> Vec<Definition> {
    const LEVELS: [&str; 6] = ["#", "##", "###", "####", "#####", "######"];
    let mut in_code = false;
    text.lines()
        .enumerate()
        .filter_map(|(row, line)| {
            if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                in_code = !in_code;
                return None;
            }
            if in_code {
                return None;
            }
            let level = line.chars().take_while(|&c| c == '#').count();
            let name = line.get(level..)?.strip_prefix(' ')?.trim().trim_end_matches('#').trim();
            ((1..=6).contains(&level) && !name.is_empty()).then(|| Definition {
                path: path.to_path_buf(),
                line: line.to_string(),
                row,
                name: name.to_string(),
                kind: LEVELS[level - 1],
            })
        })
        .collect()
}

/// What the project's `.gitignore` (and `.git/info/exclude`) leave out: build output,
/// dependencies. Their files aren't the person's code, so they stay out of the index.
pub fn ignore_rules(root: &Path) -> ignore::gitignore::Gitignore {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    builder.add(root.join(".gitignore"));
    builder.add(root.join(".git").join("info").join("exclude"));
    builder.build().unwrap_or_else(|_| ignore::gitignore::Gitignore::empty())
}

/// Whether `path` (inside `root`) is left out by the rules, itself or through a folder above it.
pub fn is_ignored(rules: &ignore::gitignore::Gitignore, root: &Path, path: &Path) -> bool {
    path.starts_with(root) && rules.matched_path_or_any_parents(path, false).is_ignore()
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
        .filter_map(|e| Some((e.path().to_path_buf(), crate::encoding::read(e.path()).ok()?.0)))
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
    fn a_markdown_file_s_symbols_are_its_headings() {
        let text = "# Null\n\nIntro.\n\n## Building\n\n```sh\n# not a heading\n```\n\n### The Mac app ##\n#tag\n";
        let found: Vec<(usize, String, &str)> =
            definitions_in(Path::new("README.md"), text).into_iter().map(|d| (d.row, d.name, d.kind)).collect();
        assert_eq!(found, [(0, "Null".into(), "#"), (4, "Building".into(), "##"), (10, "The Mac app".into(), "###")]);
    }

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
    fn ignored_files_are_recognised_through_their_folders() {
        let root = crate::tools::test_dir("ignore");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\nnode_modules\n*.gen.ts\n").unwrap();
        let rules = ignore_rules(&root);
        assert!(is_ignored(&rules, &root, &root.join("target/debug/build/x/out/a.rs")));
        assert!(is_ignored(&rules, &root, &root.join("web/node_modules/react/index.js")));
        assert!(is_ignored(&rules, &root, &root.join("src/api.gen.ts")));
        assert!(!is_ignored(&rules, &root, &root.join("src/main.rs")));
        assert!(!is_ignored(&rules, &root, Path::new("/elsewhere/a.rs")));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn definitions_know_their_name_kind_and_line() {
        let found = |path: &str, text: &str| -> Vec<(String, &'static str, usize)> {
            definitions_in(Path::new(path), text).into_iter().map(|d| (d.name, d.kind, d.row)).collect()
        };
        assert_eq!(
            found(
                "a.rs",
                "use x;\npub async fn parse(s: &str) {}\nimpl A {\n    fn len(&self) {}\n}\npub struct Ast;\nconst MAX: u8 = 1;\n"
            ),
            [
                ("parse".into(), "fn", 1),
                ("len".into(), "fn", 3),
                ("Ast".into(), "struct", 5),
                ("MAX".into(), "const", 6)
            ]
        );
        assert_eq!(
            found("a.py", "class Store:\n    def load(self):\n"),
            [("Store".into(), "class", 0), ("load".into(), "def", 1)]
        );
        assert_eq!(
            found("a.go", "func (s *Server) Start() error {\nfunc main() {\n"),
            [("Start".into(), "func", 0), ("main".into(), "func", 1)]
        );
        assert_eq!(
            found("a.c", "#define BUF 64\ntypedef struct s_list t_list;\nvoid my_putchar(char c)\n"),
            [("BUF".into(), "macro", 0), ("t_list".into(), "type", 1), ("my_putchar".into(), "fn", 2)]
        );
        assert_eq!(
            found("a.ts", "export function* gen() {}\nexport const run = () => 1;\n"),
            [("gen".into(), "function", 0), ("run".into(), "const", 1)]
        );
    }

    #[test]
    fn the_outline_starts_with_the_nearest_files() {
        let d = |p: &str, l: &str| Definition {
            path: PathBuf::from(p),
            line: l.into(),
            row: 0,
            name: String::new(),
            kind: "fn",
        };
        let defs =
            [d("/p/far/x.c", "int far(void)"), d("/p/src/near.c", "int near(void)"), d("/p/src/me.c", "int me(void)")];
        let outline = outline_for(&defs, Path::new("/p/src/me.c"), Path::new("/p"), "//");
        assert!(outline.find("near").unwrap() < outline.find("far").unwrap());
        assert!(!outline.contains("me(void)"));
    }
}
