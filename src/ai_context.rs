//! What the AI reads with a request about some code: no more than it needs. The project's
//! conventions (AGENTS.md, CLAUDE.md...), the file (whole when it's short; around the code
//! with an outline of the rest when it's long), and what that code uses from elsewhere in
//! the project, found in the project's index (no language server, nothing sent to look).
//! Put first and kept the same from one request to the next, so providers can cache it.

use crate::ai::ProviderId;
use crate::project_index::Definition;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A file this long or shorter is sent whole.
const WHOLE_FILE_LINES: usize = 400;
/// Around the code, in a longer file, this many lines each side.
const AROUND: usize = 60;
/// At most this many definitions from elsewhere.
const MAX_DEFINITIONS: usize = 12;
/// Conventions longer than this are cut.
const MAX_CONVENTIONS: usize = 6_000;

/// The files a project writes its conventions for AI in, first found used.
const CONVENTION_FILES: [&str; 4] = ["AGENTS.md", "CLAUDE.md", ".github/copilot-instructions.md", "CONVENTIONS.md"];

/// What's sent, and what it is in a few words.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub text: String,
    /// "this function's surroundings, 2 definitions, AGENTS.md".
    pub summary: String,
}

/// The project's conventions file, if it has one: its name and text (cut to size).
pub fn conventions(root: &Path) -> Option<(&'static str, String)> {
    /// Each project's, as read, and when its file was last changed then.
    type Read = (PathBuf, Option<std::time::SystemTime>, Option<(&'static str, String)>);
    static READ: Mutex<Vec<Read>> = Mutex::new(Vec::new());
    let found = CONVENTION_FILES.iter().find(|f| root.join(f).is_file());
    let modified = found.and_then(|f| std::fs::metadata(root.join(f)).ok()?.modified().ok());
    let mut read = READ.lock().ok()?;
    if let Some((_, when, kept)) = read.iter().find(|(r, _, _)| r == root)
        && *when == modified
    {
        return kept.clone();
    }
    let kept = found.and_then(|name| {
        let text = std::fs::read_to_string(root.join(name)).ok()?;
        let text: String = text.chars().take(MAX_CONVENTIONS).collect();
        (!text.trim().is_empty()).then_some((*name, text))
    });
    read.retain(|(r, _, _)| r != root);
    read.push((root.to_path_buf(), modified, kept.clone()));
    kept
}

/// Whether a file looks like it holds secrets (`.env`, a private key, a token's file): its
/// text is never sent to AI, whatever's asked. (`.env.example` and the like hold none.)
pub fn holds_secrets(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    let name = name.to_lowercase();
    let example = [".example", ".sample", ".template", ".dist"].iter().any(|e| name.ends_with(e));
    if name == ".env" || ((name.starts_with(".env.") || name.ends_with(".env")) && !example) {
        return true;
    }
    let keys = [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore", ".ppk"];
    if keys.iter().any(|k| name.ends_with(k)) {
        return true;
    }
    let private_keys = ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"];
    if private_keys.iter().any(|k| name == *k || (name.starts_with(&format!("{k}.")) && !name.ends_with(".pub"))) {
        return true;
    }
    [".netrc", ".npmrc", ".pypirc", ".pgpass", "credentials", "credentials.json", ".git-credentials", "secrets.json"]
        .contains(&name.as_str())
}

/// What to send with a request about `lines` of `text` (the file at `path`).
pub fn for_request(
    path: Option<&Path>,
    language: &str,
    text: &str,
    lines: &Range<usize>,
    root: Option<&Path>,
    definitions: &[Definition],
    provider: ProviderId,
) -> Context {
    let mut parts = Vec::new();
    let mut said = Vec::new();
    // The command line tools read the project's conventions themselves.
    if !provider.is_cli()
        && let Some((name, rules)) = root.and_then(conventions)
    {
        parts.push(format!("The project's conventions ({name}):\n<conventions>\n{}\n</conventions>", rules.trim()));
        said.push(name.to_string());
    }
    let shown_path = match (path, root) {
        (Some(p), Some(r)) => p.strip_prefix(r).unwrap_or(p).display().to_string(),
        (Some(p), None) => p.display().to_string(),
        _ => "untitled".to_string(),
    };
    let all: Vec<&str> = text.lines().collect();
    let window = if all.len() <= WHOLE_FILE_LINES {
        parts.push(format!("File: {shown_path} ({language})\n<file>\n{text}\n</file>"));
        said.push("the file".to_string());
        0..all.len()
    } else {
        let window = lines.start.saturating_sub(AROUND)..(lines.end + AROUND).min(all.len());
        let outline: Vec<String> = crate::project_index::definitions_in(path.unwrap_or(Path::new("x")), text)
            .into_iter()
            .filter(|d| !window.contains(&d.row))
            .map(|d| format!("{}: {}", d.row + 1, d.line.trim()))
            .collect();
        let mut file = format!(
            "File: {shown_path} ({language}), {} lines. Lines {}-{}:\n<file>\n{}\n</file>",
            all.len(),
            window.start + 1,
            window.end,
            all[window.clone()].join("\n")
        );
        if !outline.is_empty() {
            file.push_str(&format!("\nThe rest of the file defines (line: signature):\n{}", outline.join("\n")));
        }
        parts.push(file);
        said.push(format!("lines {}-{} and an outline of the rest", window.start + 1, window.end));
        window
    };
    // What the code uses that's defined elsewhere (in another file, or out of what's sent).
    let target: String = all.get(lines.start..lines.end.min(all.len())).map(|l| l.join("\n")).unwrap_or_default();
    let used = used_definitions(&target, path, &window, definitions);
    if !used.is_empty() {
        let listed: Vec<String> = used
            .iter()
            .map(|d| {
                let file = root.and_then(|r| d.path.strip_prefix(r).ok()).unwrap_or(&d.path);
                format!("{}:{}: {}", file.display(), d.row + 1, d.line.trim())
            })
            .collect();
        parts.push(format!("Defined elsewhere in the project, used by this code:\n{}", listed.join("\n")));
        said.push(if used.len() == 1 { "1 definition".to_string() } else { format!("{} definitions", used.len()) });
    }
    Context { text: parts.join("\n\n"), summary: said.join(", ") }
}

/// Definitions of the names `code` uses, from elsewhere than `window` of this file.
fn used_definitions<'a>(
    code: &str,
    path: Option<&Path>,
    window: &Range<usize>,
    definitions: &'a [Definition],
) -> Vec<&'a Definition> {
    let mut names: Vec<&str> = code
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| w.len() >= 3 && !w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .collect();
    names.sort_unstable();
    names.dedup();
    let mut found: Vec<&Definition> = Vec::new();
    for name in names {
        for d in definitions.iter().filter(|d| d.name == name) {
            let here = path == Some(d.path.as_path()) && window.contains(&d.row);
            if !here && !found.iter().any(|f| f.path == d.path && f.row == d.row) {
                found.push(d);
            }
        }
        if found.len() >= MAX_DEFINITIONS {
            break;
        }
    }
    found.truncate(MAX_DEFINITIONS);
    found
}

#[cfg(test)]
mod tests {

    #[test]
    fn files_holding_secrets_are_told() {
        for name in
            [".env", ".env.local", "prod.env", "server.pem", "id_ed25519", "id_rsa.old", ".npmrc", "credentials"]
        {
            assert!(holds_secrets(Path::new(name)), "{name}");
        }
        for name in [".env.example", ".env.template", "id_ed25519.pub", "main.rs", "environment.ts", "keys.md"] {
            assert!(!holds_secrets(Path::new(name)), "{name}");
        }
    }

    use super::*;

    fn definition(path: &str, row: usize, name: &str, line: &str) -> Definition {
        Definition { path: PathBuf::from(path), line: line.into(), row, name: name.into(), kind: "fn" }
    }

    #[test]
    fn a_short_file_goes_whole_with_what_it_uses() {
        let text = "fn total(cart: &Cart) -> Cents {\n    subtotal(cart) + shipping(cart)\n}\n";
        let defs = [
            definition("/p/src/price.rs", 3, "subtotal", "pub fn subtotal(cart: &Cart) -> Cents {"),
            definition("/p/src/price.rs", 9, "shipping", "pub fn shipping(cart: &Cart) -> Cents {"),
            definition("/p/src/unused.rs", 1, "elsewhere", "fn elsewhere()"),
        ];
        let context = for_request(
            Some(Path::new("/p/src/cart.rs")),
            "Rust",
            text,
            &(0..3),
            Some(Path::new("/p")),
            &defs,
            ProviderId::Claude,
        );
        assert!(context.text.contains("File: src/cart.rs (Rust)\n<file>\nfn total"));
        assert!(context.text.contains("src/price.rs:4: pub fn subtotal"), "{}", context.text);
        assert!(context.text.contains("src/price.rs:10: pub fn shipping"));
        assert!(!context.text.contains("unused.rs"));
        assert_eq!(context.summary, "the file, 2 definitions");
    }

    #[test]
    fn a_long_file_goes_around_the_code_with_an_outline_of_the_rest() {
        let mut text = String::new();
        for i in 0..900 {
            text.push_str(&if i % 100 == 0 { format!("fn part_{i}() {{\n") } else { format!("    let x{i} = {i};\n") });
        }
        let context = for_request(
            Some(Path::new("/p/big.rs")),
            "Rust",
            &text,
            &(500..505),
            Some(Path::new("/p")),
            &[],
            ProviderId::Claude,
        );
        assert!(context.text.contains("900 lines. Lines 441-565:"), "{}", &context.text[..200]);
        assert!(context.text.contains("1: fn part_0()") && context.text.contains("801: fn part_800()"));
        assert!(!context.text.contains("let x10 = 10;"), "far away lines aren't sent");
        assert_eq!(context.summary, "lines 441-565 and an outline of the rest");
        assert!(context.text.len() < text.len() / 4);
    }

    #[test]
    fn the_project_s_conventions_come_first_except_for_the_tools_that_read_them() {
        let root = crate::tools::test_dir("ai-context-conventions");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("AGENTS.md"), "Use tabs.\n").unwrap();
        let ask = |provider| for_request(None, "Rust", "fn a() {}\n", &(0..1), Some(&root), &[], provider);
        let api = ask(ProviderId::Mistral);
        assert!(api.text.starts_with("The project's conventions (AGENTS.md):\n<conventions>\nUse tabs."));
        assert_eq!(api.summary, "AGENTS.md, the file");
        assert!(!ask(ProviderId::ClaudeCode).text.contains("Use tabs"));
        // Changed: read again.
        std::fs::write(root.join("AGENTS.md"), "Use spaces.\n").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        let file = std::fs::File::options().write(true).open(root.join("AGENTS.md")).unwrap();
        file.set_modified(later).unwrap();
        assert!(ask(ProviderId::Mistral).text.contains("Use spaces."));
    }
}
