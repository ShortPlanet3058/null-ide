//! Your own snippets, written the way VS Code writes them so they carry over: a file a
//! language in Null's snippets folder (`~/.config/null/snippets/rust.json`), and files for
//! any language there or in a project's `.vscode` folder (`*.code-snippets`, each snippet
//! with an optional "scope"). Typing a snippet's prefix offers it among the suggestions;
//! picked, it becomes its body, ⇥ going from one place to fill in to the next.

use lsp_types::{CompletionItem, CompletionItemKind, InsertTextFormat};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

/// One snippet as written.
#[derive(Clone, Debug, PartialEq)]
pub struct Snippet {
    pub name: String,
    pub prefixes: Vec<String>,
    pub body: String,
    pub description: Option<String>,
    /// The languages it's for (VS Code's ids), when its file says; None: any.
    scope: Option<Vec<String>>,
}

/// Null's own snippets folder, next to the settings.
pub fn folder() -> Option<PathBuf> {
    crate::settings::Settings::path()?.parent().map(|dir| dir.join("snippets"))
}

/// VS Code's id for a file's language, as Null knows it (`.zshrc` is Shell): what its
/// snippets file is named after. By its extension when Null has no grammar for it.
pub fn language_id(language: Option<&str>, path: &Path) -> &'static str {
    let jsx = path.extension().is_some_and(|e| e == "jsx");
    match language {
        Some("Rust") => "rust",
        Some("CSS") => "css",
        Some("Go") => "go",
        Some("C") => "c",
        Some("C++") => "cpp",
        Some("Python") => "python",
        Some("JavaScript") if jsx => "javascriptreact",
        Some("JavaScript") => "javascript",
        Some("TypeScript") => "typescript",
        Some("TSX") => "typescriptreact",
        Some("TOML") => "toml",
        Some("Markdown") => "markdown",
        Some("HTML") => "html",
        Some("YAML") => "yaml",
        Some("Shell") => "shellscript",
        _ => crate::servers::language_id(path),
    }
}

/// The snippets for a file in `language`: its file in `folder`, then the `*.code-snippets`
/// there and in the project's `.vscode` folder that are for its language or any.
pub fn for_file(path: &Path, language: &str, folder: Option<&Path>) -> Vec<Snippet> {
    let mut files: Vec<(PathBuf, bool)> = Vec::new();
    if let Some(folder) = folder {
        files.push((folder.join(format!("{language}.json")), false));
        files.extend(code_snippets_in(folder).into_iter().map(|f| (f, true)));
    }
    if let Some(vscode) = project_vscode(path) {
        files.extend(code_snippets_in(&vscode).into_iter().map(|f| (f, true)));
    }
    files
        .iter()
        .flat_map(|&(ref file, scoped)| {
            read_cached(file)
                .into_iter()
                .filter(move |s| !scoped || s.scope.as_ref().is_none_or(|scope| scope.iter().any(|l| l == language)))
        })
        .collect()
}

/// The suggestion a snippet makes, indented for a line indented `indent`, with its tabs
/// as `unit` (the file's own indentation) and its lines ending as the file's do.
pub fn completion_item(snippet: &Snippet, prefix: &str, indent: &str, unit: &str, ending: &str) -> CompletionItem {
    let body = snippet
        .body
        .split('\n')
        .enumerate()
        .map(|(i, line)| {
            let tabs = line.len() - line.trim_start_matches('\t').len();
            let line = format!("{}{}", unit.repeat(tabs), &line[tabs..]);
            if i == 0 { line } else { format!("{indent}{line}") }
        })
        .collect::<Vec<_>>()
        .join(ending);
    CompletionItem {
        label: prefix.to_string(),
        kind: Some(CompletionItemKind::SNIPPET),
        detail: Some(snippet.description.clone().unwrap_or_else(|| snippet.name.clone())),
        // Matched on its words: `#region` by what's typed after the `#`.
        filter_text: Some(prefix.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '_').to_string()),
        sort_text: Some(prefix.to_string()),
        insert_text: Some(body),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    }
}

/// The `.vscode` folder of the project a file is in: in its folder or one above, not past
/// the top of its git repository.
fn project_vscode(path: &Path) -> Option<PathBuf> {
    for dir in path.ancestors().skip(1) {
        let vscode = dir.join(".vscode");
        if vscode.is_dir() {
            return Some(vscode);
        }
        if dir.join(".git").exists() {
            return None;
        }
    }
    None
}

fn code_snippets_in(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "code-snippets"))
        .collect();
    files.sort();
    files
}

/// A file's snippets, and when the file was last changed when they were read.
type Read = (SystemTime, Vec<Snippet>);

/// Files read: each is read again only once changed.
static READ: LazyLock<Mutex<HashMap<PathBuf, Read>>> = LazyLock::new(Default::default);

fn read_cached(file: &Path) -> Vec<Snippet> {
    let Some(modified) = std::fs::metadata(file).and_then(|m| m.modified()).ok() else { return Vec::new() };
    let mut read = READ.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((when, snippets)) = read.get(file)
        && *when == modified
    {
        return snippets.clone();
    }
    let snippets = std::fs::read_to_string(file).map(|text| parse(&text)).unwrap_or_default();
    read.insert(file.to_path_buf(), (modified, snippets.clone()));
    snippets
}

/// The snippets in a file's text: JSON with comments and trailing commas allowed, as VS
/// Code allows them. Ones without a prefix or a body are left out.
pub fn parse(text: &str) -> Vec<Snippet> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Ok(serde_json::Value::Object(entries)) = serde_json::from_str(&without_comments(text)) else {
        return Vec::new();
    };
    let strings = |v: Option<&serde_json::Value>| -> Vec<String> {
        match v {
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            Some(serde_json::Value::Array(items)) => {
                items.iter().filter_map(|i| i.as_str().map(String::from)).collect()
            }
            _ => Vec::new(),
        }
    };
    let mut snippets: Vec<Snippet> = entries
        .iter()
        .filter_map(|(name, entry)| {
            let prefixes: Vec<String> = strings(entry.get("prefix")).into_iter().filter(|p| !p.is_empty()).collect();
            let body = strings(entry.get("body"));
            (!prefixes.is_empty() && !body.is_empty()).then(|| Snippet {
                name: name.clone(),
                prefixes,
                body: body.join("\n"),
                description: entry.get("description").and_then(|d| d.as_str()).map(String::from),
                scope: entry
                    .get("scope")
                    .and_then(|s| s.as_str())
                    .map(|s| s.split(',').map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()),
            })
        })
        .collect();
    snippets.sort_by(|a, b| a.name.cmp(&b.name));
    snippets
}

/// `text` without `//` and `/* */` comments, nor commas before a closing bracket.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => while chars.next_if(|&n| n != '\n').is_some() {},
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for n in chars.by_ref() {
                    if last == '*' && n == '/' {
                        break;
                    }
                    last = n;
                }
            }
            (']' | '}', _) => {
                // A comma before it, past blanks, goes.
                let kept = out.trim_end().len();
                if out[..kept].ends_with(',') {
                    out.truncate(kept - 1);
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// What a new snippets file for `language` starts as: how to write one, and nothing yet.
pub fn new_file(language: &str) -> String {
    format!(
        "// Your snippets for {language} files. Each one has a name, the prefix you type, and the\n\
         // body it becomes; $1, $2… are places to fill in (⇥ goes to the next) and $0 is where\n\
         // the caret ends. For example:\n\
         //\n\
         //   \"Print a value\": {{\n\
         //     \"prefix\": \"pv\",\n\
         //     \"body\": [\"print(${{1:value}})$0\"],\n\
         //     \"description\": \"Print a value\"\n\
         //   }}\n\
         {{\n\
         }}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_are_read_as_vscode_writes_them() {
        let text = r#"{
            // A comment, and a trailing comma: VS Code allows both.
            "Log": { "prefix": ["log", "cl"], "body": ["console.log($1);", "$0"], "description": "Log it", },
            "Url": { "prefix": "url", "body": "https://example.com/* not a comment */", "scope": "javascript, typescript" },
            /* no body */ "Broken": { "prefix": "x" },
        }"#;
        let snippets = parse(text);
        assert_eq!(snippets.len(), 2, "{snippets:?}");
        assert_eq!(snippets[0].name, "Log");
        assert_eq!(snippets[0].prefixes, ["log", "cl"]);
        assert_eq!(snippets[0].body, "console.log($1);\n$0");
        assert_eq!(snippets[1].body, "https://example.com/* not a comment */");
        assert_eq!(snippets[1].scope.as_deref(), Some(&["javascript".to_string(), "typescript".into()][..]));
        assert!(parse("not json").is_empty());
        assert_eq!(parse("\u{feff}{\"A\": {\"prefix\": \"a\", \"body\": \"b\"}}").len(), 1, "a byte-order mark");
        let zshrc = std::path::Path::new(".zshrc");
        assert_eq!(language_id(crate::languages::for_path(zshrc).map(|l| l.name), zshrc), "shellscript");
        assert_eq!(language_id(None, std::path::Path::new("a.weird")), "plaintext");
        assert!(parse(&new_file("Rust")).is_empty(), "the new file's example is a comment");
    }

    #[test]
    fn snippets_come_from_the_language_file_and_scoped_files() {
        let dir = crate::tools::test_dir("snippets-files");
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("snippets");
        let project = dir.join("project");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(project.join(".vscode")).unwrap();
        std::fs::create_dir_all(project.join(".git")).unwrap();
        std::fs::write(folder.join("rust.json"), r#"{"Print": {"prefix": "pv", "body": "println!($1);"}}"#).unwrap();
        std::fs::write(folder.join("python.json"), r#"{"Py": {"prefix": "py", "body": "x"}}"#).unwrap();
        std::fs::write(
            folder.join("mine.code-snippets"),
            r#"{"Any": {"prefix": "any", "body": "a"}, "Js": {"prefix": "js", "body": "j", "scope": "javascript"}}"#,
        )
        .unwrap();
        std::fs::write(project.join(".vscode/team.code-snippets"), r#"{"Team": {"prefix": "team", "body": "t"}}"#)
            .unwrap();
        let names = |file: &str| -> Vec<String> {
            let path = project.join(file);
            let language = crate::languages::for_path(&path).map(|l| l.name);
            for_file(&path, language_id(language, &path), Some(&folder)).into_iter().map(|s| s.name).collect()
        };
        assert_eq!(names("src/main.rs"), ["Print", "Any", "Team"]);
        assert_eq!(names("app.js"), ["Any", "Js", "Team"]);
        // A changed file is read again.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(folder.join("rust.json"), r#"{"Debug": {"prefix": "dbg", "body": "dbg!($1)"}}"#).unwrap();
        assert_eq!(names("src/main.rs")[0], "Debug");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_snippet_takes_the_lines_indentation() {
        let snippet = Snippet {
            name: "If".into(),
            prefixes: vec!["if".into()],
            body: "if $1 {\n\t$0\n}".into(),
            description: None,
            scope: None,
        };
        let item = completion_item(&snippet, "if", "    ", "    ", "\n");
        assert_eq!(item.insert_text.as_deref(), Some("if $1 {\n        $0\n    }"));
        let windows = completion_item(&snippet, "if", "", "\t", "\r\n");
        assert_eq!(windows.insert_text.as_deref(), Some("if $1 {\r\n\t$0\r\n}"));
        assert_eq!(item.label, "if");
        assert_eq!(item.detail.as_deref(), Some("If"));
    }
}
