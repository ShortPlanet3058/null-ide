//! What a project can run, read from its own files: Cargo's commands, package.json's
//! scripts (with the package manager its lockfile names), Makefile and justfile
//! targets, Go's commands. Run in the terminal from ⌘⇧B.

use std::path::Path;

/// Something to run: what the list shows, and the command typed into the terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTask {
    pub label: String,
    pub command: String,
    /// Where it comes from ("Cargo", "package.json", "Makefile"…), shown beside it.
    pub source: &'static str,
}

fn task(command: impl Into<String>, source: &'static str) -> ProjectTask {
    let command = command.into();
    ProjectTask { label: command.clone(), command, source }
}

/// Every task found in `root`, in a sensible order: build tools first.
pub fn find(root: &Path) -> Vec<ProjectTask> {
    let mut found = Vec::new();
    if root.join("Cargo.toml").is_file() {
        for command in ["cargo run", "cargo build", "cargo test", "cargo check", "cargo clippy"] {
            found.push(task(command, "Cargo"));
        }
    }
    if let Ok(text) = std::fs::read_to_string(root.join("package.json")) {
        let runner = package_runner(root);
        for name in package_scripts(&text) {
            found.push(task(format!("{runner} run {name}"), "package.json"));
        }
    }
    if root.join("go.mod").is_file() {
        for command in ["go run .", "go build ./...", "go test ./..."] {
            found.push(task(command, "Go"));
        }
    }
    for file in ["Makefile", "makefile", "GNUmakefile"] {
        if let Ok(text) = std::fs::read_to_string(root.join(file)) {
            found.extend(make_targets(&text).into_iter().map(|t| task(format!("make {t}"), "Makefile")));
            break;
        }
    }
    for file in ["justfile", "Justfile", ".justfile"] {
        if let Ok(text) = std::fs::read_to_string(root.join(file)) {
            found.extend(make_targets(&text).into_iter().map(|t| task(format!("just {t}"), "justfile")));
            break;
        }
    }
    found
}

/// The package manager a JavaScript project uses, by its lockfile.
fn package_runner(root: &Path) -> &'static str {
    if root.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if root.join("yarn.lock").is_file() {
        "yarn"
    } else if root.join("bun.lockb").is_file() || root.join("bun.lock").is_file() {
        "bun"
    } else {
        "npm"
    }
}

/// The names of package.json's scripts, in the file's order.
fn package_scripts(text: &str) -> Vec<String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else { return Vec::new() };
    // serde_json keeps keys sorted; the file's own order reads better, so find each in the text.
    let mut names: Vec<String> = scripts.keys().cloned().collect();
    names.sort_by_key(|name| text.find(&format!("\"{name}\"")).unwrap_or(usize::MAX));
    names
}

/// Targets of a Makefile (or recipes of a justfile): names at the start of a line
/// followed by `:`, leaving out special and pattern targets and variable assignments.
fn make_targets(text: &str) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with([' ', '\t', '#', '.']) {
            continue;
        }
        let Some((head, rest)) = line.split_once(':') else { continue };
        // `x := 1` and `x ::= 1` are assignments, not targets.
        if rest.starts_with('=') || rest.starts_with(":=") {
            continue;
        }
        // A justfile recipe can take parameters: `test name:`. The first word names it.
        let Some(name) = head.split_whitespace().next() else { continue };
        let plain = name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/'));
        if plain && !targets.iter().any(|t| t == name) {
            targets.push(name.to_string());
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_scripts_in_the_file_s_order() {
        let text = r#"{ "name": "x", "scripts": { "dev": "vite", "build": "vite build", "test": "vitest" } }"#;
        assert_eq!(package_scripts(text), ["dev", "build", "test"]);
        assert!(package_scripts("{}").is_empty());
    }

    #[test]
    fn reads_make_and_just_targets() {
        let make =
            "CC := gcc\n.PHONY: all\nall: build\n\nbuild: main.o\n\t$(CC) main.o\n%.o: %.c\n\tcc -c $<\nclean:\n";
        assert_eq!(make_targets(make), ["all", "build", "clean"]);
        let just = "set shell := [\"bash\"]\n# Runs the tests\ntest filter='':\n    cargo test {{filter}}\nfmt:\n    cargo fmt\n";
        assert_eq!(make_targets(just), ["test", "fmt"]);
    }

    #[test]
    fn finds_a_project_s_tasks() {
        let dir = std::env::temp_dir().join(format!("null-tasks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package.json"), r#"{"scripts":{"dev":"vite"}}"#).unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(dir.join("Makefile"), "deploy:\n\t./deploy.sh\n").unwrap();
        let commands: Vec<String> = find(&dir).into_iter().map(|t| t.command).collect();
        assert_eq!(commands, ["pnpm run dev", "make deploy"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
