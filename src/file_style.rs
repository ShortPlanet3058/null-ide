//! How a file is written, so editing keeps to it: its indentation (tabs, or how many
//! spaces), its line endings, and what saving tidies. From the project's `.editorconfig`
//! first, then from the file itself, then from the settings.

use regex::Regex;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    Tabs,
    Spaces(usize),
}

impl Indent {
    /// One level of indentation as text.
    pub fn unit(self) -> String {
        match self {
            Indent::Tabs => "\t".into(),
            Indent::Spaces(n) => " ".repeat(n),
        }
    }

    /// Columns per level.
    pub fn width(self) -> usize {
        match self {
            Indent::Tabs => crate::editor::TAB_SIZE,
            Indent::Spaces(n) => n,
        }
    }

    pub fn label(self) -> String {
        match self {
            Indent::Tabs => "Tabs".into(),
            Indent::Spaces(n) => format!("{n} spaces"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    pub fn text(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStyle {
    pub indent: Indent,
    pub line_ending: LineEnding,
    /// Saving makes sure the file ends with a line break (or that it doesn't).
    pub final_newline: Option<bool>,
    /// Saving removes spaces at the ends of lines.
    pub trim_trailing: bool,
    /// The line length the project keeps to, when it says (a faint guide shows it).
    pub ruler: Option<usize>,
}

impl Default for FileStyle {
    fn default() -> Self {
        Self {
            indent: Indent::Spaces(4),
            line_ending: LineEnding::Lf,
            final_newline: None,
            trim_trailing: false,
            ruler: None,
        }
    }
}

impl FileStyle {
    /// The style for a file: `.editorconfig` says first, then the file's own text, then
    /// `default` (with a language's usual indentation where it has one).
    pub fn for_file(path: Option<&Path>, text: &str, default: Indent) -> Self {
        let config = path.map(editorconfig).unwrap_or_default();
        // Make needs tabs, whatever's set (Go's are only its way: see `Settings::indent_for`).
        let language_default = match path.and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
            Some(name) if name == "Makefile" || name.ends_with(".mk") => Indent::Tabs,
            _ => default,
        };
        Self {
            indent: config.indent.or_else(|| detect_indent(text)).unwrap_or(language_default),
            line_ending: config.line_ending.unwrap_or_else(|| detect_line_ending(text)),
            final_newline: config.final_newline,
            trim_trailing: config.trim_trailing.unwrap_or(false),
            ruler: config.max_line_length.or_else(|| path.and_then(formatter_width)),
        }
    }
}

/// The line length a file's formatter is set to keep to, from the project's own config:
/// rustfmt's `max_width`, Prettier's `printWidth`, Black's or Ruff's `line-length`. Only
/// what's written down: a formatter's default draws no guide.
pub fn formatter_width(path: &Path) -> Option<usize> {
    let extension = path.extension()?.to_str()?.to_lowercase();
    let files: &[(&str, &str)] = match extension.as_str() {
        "rs" => &[("rustfmt.toml", "max_width"), (".rustfmt.toml", "max_width")],
        "py" | "pyi" => {
            &[("pyproject.toml", "line-length"), ("ruff.toml", "line-length"), (".ruff.toml", "line-length")]
        }
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts" | "css" | "scss" | "less" | "html" | "vue"
        | "svelte" | "json" | "md" | "yaml" | "yml" => &[
            (".prettierrc", "printWidth"),
            (".prettierrc.json", "printWidth"),
            (".prettierrc.yaml", "printWidth"),
            (".prettierrc.yml", "printWidth"),
            ("package.json", "printWidth"),
        ],
        _ => return None,
    };
    for dir in path.ancestors().skip(1) {
        for (name, key) in files {
            let Ok(text) = std::fs::read_to_string(dir.join(name)) else { continue };
            if let Some(width) = number_after(&text, key) {
                return Some(width);
            }
        }
        // The project's top: no further up.
        if dir.join(".git").exists() {
            break;
        }
    }
    None
}

/// The number set for `key` in a TOML, JSON or YAML text: `key = 100`, `"key": 100`,
/// `key: 100`, wherever it is (a JSON file can be all on one line).
fn number_after(text: &str, key: &str) -> Option<usize> {
    text.match_indices(key).find_map(|(at, _)| {
        // The whole key, not the end of a longer one (`max-line-length`, `my_max_width`).
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return None;
        }
        let rest = text[at + key.len()..].trim_start_matches('"').trim_start();
        let rest = rest.strip_prefix('=').or_else(|| rest.strip_prefix(':'))?.trim_start();
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok().filter(|&n: &usize| (20..=400).contains(&n))
    })
}

/// The indentation a text uses, if it shows: tabs or spaces, whichever most lines start
/// with; for spaces, the step most lines go in by from the line above.
/// The indentation a file shows itself: its `.editorconfig`'s, or the one it's written in.
pub fn own_indent(path: Option<&Path>, text: &str) -> Option<Indent> {
    path.and_then(|p| editorconfig(p).indent).or_else(|| detect_indent(text))
}

pub fn detect_indent(text: &str) -> Option<Indent> {
    let (mut tabs, mut spaces) = (0, 0);
    let mut steps = [0usize; 9];
    let mut previous = 0;
    for line in text.lines().take(2000) {
        if line.trim().is_empty() {
            continue;
        }
        // Block comment continuations (" * text") are indented by one: not a step.
        let trimmed = line.trim_start();
        if trimmed.starts_with('*') && !trimmed.starts_with("*/") && line.starts_with(' ') {
            continue;
        }
        if line.starts_with('\t') {
            tabs += 1;
            previous = 0;
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent > 0 {
            spaces += 1;
        }
        if indent > previous && indent - previous <= 8 {
            steps[indent - previous] += 1;
        }
        previous = indent;
    }
    if tabs == 0 && spaces == 0 {
        return None;
    }
    if tabs > spaces {
        return Some(Indent::Tabs);
    }
    // The step lines go in by most often (a tie goes to the wider one). Without any step,
    // it can't tell.
    let (step, count) = steps.iter().enumerate().skip(2).max_by_key(|(n, c)| (**c, *n))?;
    (*count > 0).then_some(Indent::Spaces(step))
}

/// CRLF when most line breaks are, else LF.
pub fn detect_line_ending(text: &str) -> LineEnding {
    let total = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    if total > 0 && crlf * 2 > total { LineEnding::Crlf } else { LineEnding::Lf }
}

/// What `.editorconfig` files say about a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorConfig {
    pub indent: Option<Indent>,
    pub line_ending: Option<LineEnding>,
    pub final_newline: Option<bool>,
    pub trim_trailing: Option<bool>,
    pub max_line_length: Option<usize>,
}

/// The `.editorconfig` settings for `path`: files from its folder up to the one marked
/// `root = true`, nearer ones winning, later sections winning within a file.
pub fn editorconfig(path: &Path) -> EditorConfig {
    let mut files = Vec::new();
    let mut dir = path.parent();
    while let Some(d) = dir {
        if let Ok(text) = std::fs::read_to_string(d.join(".editorconfig")) {
            let root = text.lines().any(|l| {
                let l = l.trim().to_lowercase().replace(' ', "");
                l == "root=true"
            });
            files.push((d.to_path_buf(), text));
            if root {
                break;
            }
        }
        dir = d.parent();
    }
    let mut config = EditorConfig::default();
    let (mut style, mut size) = (None::<String>, None::<String>);
    // Farthest first, so nearer files override.
    for (dir, text) in files.iter().rev() {
        let Ok(relative) = path.strip_prefix(dir) else { continue };
        let relative = relative.to_string_lossy().replace('\\', "/");
        let mut applies = false;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                applies = glob_matches(section, &relative);
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            if !applies {
                continue;
            }
            let (key, value) = (key.trim().to_lowercase(), value.trim().to_lowercase());
            let flag = match value.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
            match key.as_str() {
                "indent_style" => style = Some(value),
                "indent_size" => size = Some(value),
                "end_of_line" => {
                    config.line_ending = match value.as_str() {
                        "crlf" => Some(LineEnding::Crlf),
                        "lf" => Some(LineEnding::Lf),
                        _ => config.line_ending,
                    }
                }
                "insert_final_newline" => config.final_newline = flag.or(config.final_newline),
                "trim_trailing_whitespace" => config.trim_trailing = flag.or(config.trim_trailing),
                // "off" takes back a length set further up.
                "max_line_length" => config.max_line_length = value.parse().ok(),
                _ => {}
            }
        }
    }
    config.indent = match (style.as_deref(), size.as_deref().and_then(|s| s.parse::<usize>().ok())) {
        (Some("tab"), _) => Some(Indent::Tabs),
        (Some("space"), Some(n)) if (1..=16).contains(&n) => Some(Indent::Spaces(n)),
        (Some("space"), _) => Some(Indent::Spaces(4)),
        (None, Some(n)) if (1..=16).contains(&n) => Some(Indent::Spaces(n)),
        _ => None,
    };
    config
}

/// Whether an `.editorconfig` section pattern matches a path relative to its folder.
/// A pattern without a slash matches the file name at any depth.
fn glob_matches(pattern: &str, relative: &str) -> bool {
    let anywhere = !pattern.contains('/');
    let pattern = pattern.trim_start_matches('/');
    let mut regex = String::from(if anywhere { "^(?:.*/)?" } else { "^" });
    let mut chars = pattern.chars().peekable();
    let mut in_braces = false;
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                regex.push_str(".*");
            }
            '*' => regex.push_str("[^/]*"),
            '?' => regex.push_str("[^/]"),
            '{' => {
                in_braces = true;
                regex.push_str("(?:");
            }
            '}' if in_braces => {
                in_braces = false;
                regex.push(')');
            }
            ',' if in_braces => regex.push('|'),
            '[' | ']' => regex.push(c),
            c => regex.push_str(&regex::escape(&c.to_string())),
        }
    }
    regex.push('$');
    Regex::new(&regex).is_ok_and(|r| r.is_match(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation_is_read_from_the_text() {
        assert_eq!(detect_indent("a {\n  b {\n    c\n  }\n}\n"), Some(Indent::Spaces(2)));
        assert_eq!(detect_indent("def f():\n    if x:\n        y\n"), Some(Indent::Spaces(4)));
        assert_eq!(detect_indent("func f() {\n\tx := 1\n\tif x {\n\t\ty()\n\t}\n}\n"), Some(Indent::Tabs));
        assert_eq!(detect_indent("/**\n * doc\n */\nint x;\n"), None);
        assert_eq!(detect_indent("flat\ntext\n"), None);
    }

    #[test]
    fn line_endings_follow_most_lines() {
        assert_eq!(detect_line_ending("a\r\nb\r\nc\n"), LineEnding::Crlf);
        assert_eq!(detect_line_ending("a\nb\n"), LineEnding::Lf);
        assert_eq!(detect_line_ending("one line"), LineEnding::Lf);
    }

    #[test]
    fn editorconfig_sections_match_like_editors_do() {
        assert!(glob_matches("*", "src/a.rs"));
        assert!(glob_matches("*.{js,ts}", "web/app.ts"));
        assert!(!glob_matches("*.{js,ts}", "web/app.rs"));
        assert!(glob_matches("Makefile", "sub/Makefile"));
        assert!(glob_matches("lib/**.py", "lib/x/y.py"));
        assert!(!glob_matches("lib/*.py", "lib/x/y.py"));
    }

    #[test]
    fn the_line_length_comes_from_the_project_s_config() {
        let root = crate::tools::test_dir("ruler");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let ruler = |name: &str| FileStyle::for_file(Some(&root.join(name)), "", Indent::Spaces(4)).ruler;
        // Nothing set: no guide, whatever the formatter's default.
        assert_eq!(ruler("src/main.rs"), None);
        std::fs::write(root.join("rustfmt.toml"), "max_width = 120\nuse_small_heuristics = \"Max\"\n").unwrap();
        std::fs::write(root.join("pyproject.toml"), "[tool.ruff]\nline-length = 88\n").unwrap();
        std::fs::write(root.join(".prettierrc"), "{ \"semi\": false, \"printWidth\": 100 }\n").unwrap();
        assert_eq!(ruler("src/main.rs"), Some(120));
        assert_eq!(ruler("app.py"), Some(88));
        assert_eq!(ruler("web.ts"), Some(100));
        assert_eq!(ruler("main.go"), None);
        // .editorconfig says first; "off" means no guide.
        std::fs::write(root.join(".editorconfig"), "[*.rs]\nmax_line_length = 80\n[*.py]\nmax_line_length = off\n")
            .unwrap();
        assert_eq!(ruler("src/main.rs"), Some(80));
        assert_eq!(ruler("app.py"), Some(88));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nearer_editorconfig_files_win() {
        let root = crate::tools::test_dir("editorconfig");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("web")).unwrap();
        std::fs::write(
            root.join(".editorconfig"),
            "root = true\n[*]\nindent_style = space\nindent_size = 4\ninsert_final_newline = true\n[*.md]\ntrim_trailing_whitespace = false\n",
        )
        .unwrap();
        std::fs::write(root.join("web/.editorconfig"), "[*.ts]\nindent_size = 2\nend_of_line = crlf\n").unwrap();
        let config = editorconfig(&root.join("web/app.ts"));
        assert_eq!(config.indent, Some(Indent::Spaces(2)));
        assert_eq!(config.line_ending, Some(LineEnding::Crlf));
        assert_eq!(config.final_newline, Some(true));
        let style = FileStyle::for_file(Some(&root.join("src/main.go")), "", Indent::Spaces(4));
        // The root file says 4 spaces for everything: it wins over Go's usual tabs.
        assert_eq!(style.indent, Indent::Spaces(4));
        std::fs::remove_dir_all(&root).ok();
        // Without any config, Go files use tabs: Go's way, unless set otherwise for it.
        let go = crate::settings::Settings::default().indent_for("Go");
        let style = FileStyle::for_file(Some(Path::new("/nowhere/main.go")), "", go);
        assert_eq!(style.indent, Indent::Tabs);
    }
}
