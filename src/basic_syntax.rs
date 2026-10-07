//! Colours for languages Null has no grammar for (Swift, Kotlin, Java, C#, Dart, Ruby, PHP,
//! Lua, SQL, Dockerfiles, Makefiles, XML…): a small scanner that knows each one's comments,
//! strings and keywords, and calls numbers, names followed by `(` and capitalised names what
//! they likely are. Not a grammar, but the file reads as code.

use crate::highlight::Span;
use crate::theme::Syntax;
use std::ops::Range;
use std::path::Path;

/// What the scanner needs to know about a language.
pub struct Basic {
    pub name: &'static str,
    extensions: &'static [&'static str],
    file_names: &'static [&'static str],
    pub line_comment: &'static [&'static str],
    pub block_comment: Option<(&'static str, &'static str)>,
    /// Quotes that open a string ending on the same line, and a quote run that opens one
    /// that can span lines (`"""`).
    quotes: &'static [u8],
    long_string: Option<&'static str>,
    keywords: &'static [&'static str],
    /// SQL and Dockerfiles: keywords in any case.
    any_case: bool,
    /// Capitalised names are types (most languages here).
    capital_types: bool,
}

const SWIFT: &[&str] = &[
    "let",
    "var",
    "func",
    "if",
    "else",
    "guard",
    "return",
    "for",
    "in",
    "while",
    "repeat",
    "switch",
    "case",
    "default",
    "break",
    "continue",
    "struct",
    "class",
    "enum",
    "protocol",
    "extension",
    "import",
    "init",
    "deinit",
    "self",
    "Self",
    "super",
    "nil",
    "true",
    "false",
    "try",
    "catch",
    "throw",
    "throws",
    "rethrows",
    "async",
    "await",
    "public",
    "private",
    "fileprivate",
    "internal",
    "open",
    "static",
    "final",
    "override",
    "mutating",
    "lazy",
    "weak",
    "unowned",
    "where",
    "as",
    "is",
    "some",
    "any",
    "typealias",
    "associatedtype",
    "defer",
    "do",
    "inout",
    "subscript",
    "get",
    "set",
    "willSet",
    "didSet",
    "actor",
    "nonisolated",
];
const KOTLIN: &[&str] = &[
    "as",
    "break",
    "class",
    "continue",
    "do",
    "else",
    "false",
    "for",
    "fun",
    "if",
    "in",
    "interface",
    "is",
    "null",
    "object",
    "package",
    "return",
    "super",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "val",
    "var",
    "when",
    "while",
    "by",
    "catch",
    "constructor",
    "init",
    "import",
    "finally",
    "where",
    "companion",
    "data",
    "enum",
    "sealed",
    "open",
    "override",
    "private",
    "protected",
    "public",
    "internal",
    "abstract",
    "final",
    "lateinit",
    "suspend",
    "inline",
    "reified",
    "const",
];
const JAVA: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
    "true",
    "false",
    "null",
    "var",
    "record",
    "yield",
    "sealed",
    "permits",
];
const CSHARP: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sealed",
    "short",
    "sizeof",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "using",
    "var",
    "virtual",
    "void",
    "volatile",
    "while",
    "async",
    "await",
    "get",
    "set",
    "record",
    "init",
];
const DART: &[&str] = &[
    "abstract",
    "as",
    "assert",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "dynamic",
    "else",
    "enum",
    "export",
    "extends",
    "extension",
    "factory",
    "false",
    "final",
    "finally",
    "for",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "is",
    "late",
    "library",
    "mixin",
    "new",
    "null",
    "on",
    "part",
    "required",
    "rethrow",
    "return",
    "set",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "var",
    "void",
    "while",
    "with",
    "yield",
];
const RUBY: &[&str] = &[
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "false",
    "for",
    "if",
    "in",
    "module",
    "next",
    "nil",
    "not",
    "or",
    "redo",
    "rescue",
    "retry",
    "return",
    "self",
    "super",
    "then",
    "true",
    "undef",
    "unless",
    "until",
    "when",
    "while",
    "yield",
    "require",
    "require_relative",
    "attr_accessor",
    "attr_reader",
    "attr_writer",
    "private",
    "protected",
    "public",
    "lambda",
    "proc",
];
const PHP: &[&str] = &[
    "abstract",
    "and",
    "array",
    "as",
    "break",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "enum",
    "extends",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "if",
    "implements",
    "include",
    "instanceof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "require_once",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "yield",
    "true",
    "false",
    "null",
    "self",
    "parent",
];
const LUA: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil",
    "not", "or", "repeat", "return", "then", "true", "until", "while",
];
const SQL: &[&str] = &[
    "select",
    "from",
    "where",
    "insert",
    "into",
    "values",
    "update",
    "set",
    "delete",
    "create",
    "table",
    "drop",
    "alter",
    "add",
    "join",
    "left",
    "right",
    "inner",
    "outer",
    "full",
    "on",
    "group",
    "by",
    "order",
    "having",
    "limit",
    "offset",
    "as",
    "and",
    "or",
    "not",
    "null",
    "is",
    "in",
    "like",
    "between",
    "distinct",
    "union",
    "all",
    "primary",
    "key",
    "foreign",
    "references",
    "index",
    "view",
    "case",
    "when",
    "then",
    "else",
    "end",
    "exists",
    "default",
    "unique",
    "with",
    "returning",
    "begin",
    "commit",
    "rollback",
    "integer",
    "text",
    "varchar",
    "boolean",
    "true",
    "false",
    "asc",
    "desc",
    "count",
    "sum",
    "avg",
    "min",
    "max",
];
const DOCKERFILE: &[&str] = &[
    "from",
    "run",
    "cmd",
    "copy",
    "add",
    "env",
    "arg",
    "workdir",
    "expose",
    "entrypoint",
    "user",
    "volume",
    "label",
    "healthcheck",
    "shell",
    "stopsignal",
    "onbuild",
    "as",
];

const C_COMMENTS: &[&str] = &["//"];
const HASH: &[&str] = &["#"];

static BASICS: &[Basic] = &[
    Basic {
        name: "Swift",
        extensions: &["swift"],
        file_names: &[],
        line_comment: C_COMMENTS,
        block_comment: Some(("/*", "*/")),
        quotes: b"\"",
        long_string: Some("\"\"\""),
        keywords: SWIFT,
        any_case: false,
        capital_types: true,
    },
    Basic {
        name: "Kotlin",
        extensions: &["kt", "kts"],
        file_names: &[],
        line_comment: C_COMMENTS,
        block_comment: Some(("/*", "*/")),
        quotes: b"\"'",
        long_string: Some("\"\"\""),
        keywords: KOTLIN,
        any_case: false,
        capital_types: true,
    },
    Basic {
        name: "Java",
        extensions: &["java"],
        file_names: &[],
        line_comment: C_COMMENTS,
        block_comment: Some(("/*", "*/")),
        quotes: b"\"'",
        long_string: Some("\"\"\""),
        keywords: JAVA,
        any_case: false,
        capital_types: true,
    },
    Basic {
        name: "C#",
        extensions: &["cs"],
        file_names: &[],
        line_comment: C_COMMENTS,
        block_comment: Some(("/*", "*/")),
        quotes: b"\"'",
        long_string: Some("\"\"\""),
        keywords: CSHARP,
        any_case: false,
        capital_types: true,
    },
    Basic {
        name: "Dart",
        extensions: &["dart"],
        file_names: &[],
        line_comment: C_COMMENTS,
        block_comment: Some(("/*", "*/")),
        quotes: b"\"'",
        long_string: Some("'''"),
        keywords: DART,
        any_case: false,
        capital_types: true,
    },
    Basic {
        name: "Ruby",
        extensions: &["rb", "rake", "gemspec"],
        file_names: &["Gemfile", "Rakefile"],
        line_comment: HASH,
        block_comment: Some(("=begin", "=end")),
        quotes: b"\"'",
        long_string: None,
        keywords: RUBY,
        any_case: false,
        capital_types: true,
    },
    Basic {
        name: "PHP",
        extensions: &["php"],
        file_names: &[],
        line_comment: &["//", "#"],
        block_comment: Some(("/*", "*/")),
        quotes: b"\"'",
        long_string: None,
        keywords: PHP,
        any_case: false,
        capital_types: true,
    },
    Basic {
        name: "Lua",
        extensions: &["lua"],
        file_names: &[],
        line_comment: &["--"],
        block_comment: Some(("--[[", "]]")),
        quotes: b"\"'",
        long_string: None,
        keywords: LUA,
        any_case: false,
        capital_types: false,
    },
    Basic {
        name: "SQL",
        extensions: &["sql"],
        file_names: &[],
        line_comment: &["--"],
        block_comment: Some(("/*", "*/")),
        quotes: b"'\"",
        long_string: None,
        keywords: SQL,
        any_case: true,
        capital_types: false,
    },
    Basic {
        name: "Dockerfile",
        extensions: &["dockerfile"],
        file_names: &["Dockerfile", "Containerfile"],
        line_comment: HASH,
        block_comment: None,
        quotes: b"\"'",
        long_string: None,
        keywords: DOCKERFILE,
        any_case: true,
        capital_types: false,
    },
    Basic {
        name: "Makefile",
        extensions: &["mk", "mak"],
        file_names: &["Makefile", "makefile", "GNUmakefile"],
        line_comment: HASH,
        block_comment: None,
        quotes: b"\"'",
        long_string: None,
        keywords: &["ifeq", "ifneq", "ifdef", "ifndef", "else", "endif", "include", "define", "endef", "export"],
        any_case: false,
        capital_types: false,
    },
    Basic {
        name: "XML",
        extensions: &["xml", "plist", "xsd", "xsl", "svg", "storyboard", "xib", "csproj"],
        file_names: &[],
        line_comment: &[],
        block_comment: Some(("<!--", "-->")),
        quotes: b"\"'",
        long_string: None,
        keywords: &[],
        any_case: false,
        capital_types: false,
    },
];

/// The scanner for a file Null has no grammar for, if it knows its language.
pub fn for_path(path: &Path) -> Option<&'static Basic> {
    let name = path.file_name()?.to_str()?;
    if let Some(basic) = BASICS.iter().find(|b| b.file_names.contains(&name)) {
        return Some(basic);
    }
    let ext = path.extension()?.to_str()?.to_lowercase();
    BASICS.iter().find(|b| b.extensions.contains(&ext.as_str()))
}

/// Past this size, a file isn't scanned (the scan runs from its start).
pub const MAX_BYTES: usize = 1024 * 1024;

/// The coloured spans of `text` (bytes) that touch `wanted`, in order. The scan starts at the
/// top, so comments and strings spanning lines are known.
pub fn spans(text: &[u8], wanted: Range<usize>, basic: &Basic) -> Vec<Span> {
    let mut found: Vec<Span> = Vec::new();
    let mut push = |range: Range<usize>, syntax: Syntax| {
        if range.end > wanted.start && range.start < wanted.end {
            found.push((range, syntax));
        }
    };
    let starts = |at: usize, token: &str| text[at..].starts_with(token.as_bytes());
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80;
    // The bytes a comment or a string can start with: above the part asked for, the others
    // are passed over at once.
    let mut opens = [false; 256];
    let markers =
        basic.line_comment.iter().chain(basic.block_comment.iter().map(|(open, _)| open)).chain(&basic.long_string);
    for marker in markers {
        opens[marker.as_bytes()[0] as usize] = true;
    }
    for &quote in basic.quotes {
        opens[quote as usize] = true;
    }
    let mut i = 0;
    let end = wanted.end.min(text.len());
    while i < end {
        let b = text[i];
        if i < wanted.start && !opens[b as usize] {
            // A name's run as one, so a quote inside it (`don't` in Ruby) isn't read as opening.
            if ident(b) {
                while i < text.len() && ident(text[i]) {
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }
        // Comments.
        if let Some((open, close)) = basic.block_comment
            && starts(i, open)
        {
            let to = find(text, i + open.len(), close).map_or(text.len(), |at| at + close.len());
            push(i..to, Syntax::Comment);
            i = to;
            continue;
        }
        if basic.line_comment.iter().any(|mark| starts(i, mark)) {
            let to = text[i..].iter().position(|&c| c == b'\n').map_or(text.len(), |at| i + at);
            push(i..to, Syntax::Comment);
            i = to;
            continue;
        }
        // Strings.
        if let Some(long) = basic.long_string
            && starts(i, long)
        {
            let to = find(text, i + long.len(), long).map_or(text.len(), |at| at + long.len());
            push(i..to, Syntax::String);
            i = to;
            continue;
        }
        if basic.quotes.contains(&b) {
            let mut j = i + 1;
            while j < text.len() && text[j] != b && text[j] != b'\n' {
                j += if text[j] == b'\\' { 2 } else { 1 };
            }
            // Its closing quote, or (left open) up to the line's end.
            let to = if text.get(j) == Some(&b) { j + 1 } else { j.min(text.len()) };
            push(i..to, Syntax::String);
            i = to;
            continue;
        }
        // Numbers, not inside a name.
        if b.is_ascii_digit() && (i == 0 || !ident(text[i - 1])) {
            let mut j = i;
            while j < text.len() && (ident(text[j]) || text[j] == b'.') {
                j += 1;
            }
            push(i..j, Syntax::Number);
            i = j;
            continue;
        }
        // Names: a keyword, a call, a type.
        if ident(b) && !b.is_ascii_digit() {
            let mut j = i;
            while j < text.len() && ident(text[j]) {
                j += 1;
            }
            // Before the part asked for, only where comments and strings are matters.
            if j <= wanted.start {
                i = j;
                continue;
            }
            let word = &text[i..j];
            let is = |k: &&str| {
                if basic.any_case { word.eq_ignore_ascii_case(k.as_bytes()) } else { word == k.as_bytes() }
            };
            let next = text[j..].iter().find(|c| **c != b' ').copied();
            if basic.keywords.iter().any(is) {
                push(i..j, Syntax::Keyword);
            } else if next == Some(b'(') {
                push(i..j, Syntax::Function);
            } else if basic.capital_types && b.is_ascii_uppercase() {
                push(i..j, Syntax::Type);
            }
            i = j;
            continue;
        }
        i += 1;
    }
    found
}

fn find(text: &[u8], from: usize, token: &str) -> Option<usize> {
    let token = token.as_bytes();
    (from..text.len().saturating_sub(token.len() - 1)).find(|&at| text[at..].starts_with(token))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coloured(file: &str, text: &str) -> Vec<(String, Syntax)> {
        let basic = for_path(Path::new(file)).unwrap();
        spans(text.as_bytes(), 0..text.len(), basic).into_iter().map(|(r, s)| (text[r].to_string(), s)).collect()
    }

    #[test]
    fn swift_reads_as_code() {
        let text = "import SwiftUI\n/* a\n note */\nstruct Card: View {\n    let n = 42 // count\n    var body: some View { Text(\"hi \\\"x\\\"\") }\n}\n";
        let got = coloured("Card.swift", text);
        let expected = [
            ("import", Syntax::Keyword),
            ("SwiftUI", Syntax::Type),
            ("/* a\n note */", Syntax::Comment),
            ("struct", Syntax::Keyword),
            ("Card", Syntax::Type),
            ("View", Syntax::Type),
            ("let", Syntax::Keyword),
            ("42", Syntax::Number),
            ("// count", Syntax::Comment),
            ("var", Syntax::Keyword),
            ("some", Syntax::Keyword),
            ("View", Syntax::Type),
            ("Text", Syntax::Function),
            ("\"hi \\\"x\\\"\"", Syntax::String),
        ];
        assert_eq!(got, expected.map(|(t, s)| (t.to_string(), s)));
    }

    /// What a keystroke costs near the end of a big file (the scan runs from its start).
    /// `cargo test timing_basic -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn timing_basic_colouring() {
        let line = "    let value = compute(\"text\", 42) // a note about it\n";
        let text = line.repeat(1024 * 1024 / line.len());
        let basic = for_path(Path::new("a.swift")).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..10 {
            let bytes: Vec<u8> = text.bytes().collect();
            let end = bytes.len();
            spans(&bytes, end - 5000..end, basic);
        }
        println!("basic colouring at the end of {} KB: {:?} a keystroke", text.len() / 1024, start.elapsed() / 10);
    }

    #[test]
    fn languages_by_name_and_their_own_rules() {
        assert_eq!(for_path(Path::new("Dockerfile")).map(|b| b.name), Some("Dockerfile"));
        assert_eq!(for_path(Path::new("src/Main.kt")).map(|b| b.name), Some("Kotlin"));
        assert!(for_path(Path::new("a.rs")).is_none());
        // SQL: keywords in any case, `--` comments, no types.
        assert_eq!(
            coloured("q.sql", "SELECT Name FROM t -- all\n"),
            [
                ("SELECT".to_string(), Syntax::Keyword),
                ("FROM".into(), Syntax::Keyword),
                ("-- all".into(), Syntax::Comment)
            ]
        );
        // Only what touches the range asked for, a comment from above it known.
        let text = "/* long\ncomment */ let x\n";
        let basic = for_path(Path::new("a.swift")).unwrap();
        let tail = spans(text.as_bytes(), 9..text.len(), basic);
        assert_eq!(tail[0], (0..18, Syntax::Comment));
        assert_eq!(tail[1].1, Syntax::Keyword);
        // A number inside a name isn't one; a quote left open ends at the line's end.
        assert_eq!(coloured("a.lua", "x2 = 'open\n"), [("'open".to_string(), Syntax::String)]);
    }
}
