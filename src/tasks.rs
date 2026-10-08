//! What a project can run, read from its own files: Cargo's commands, package.json's
//! scripts (with the package manager its lockfile names), Makefile and justfile
//! targets, Go's commands, the tasks of a `.vscode/tasks.json`; and your own, from
//! `"tasks"` in settings.json. Run in the terminal from ⌘⇧B.

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

/// Every task found in `root`, in a sensible order: the project's own list and yours
/// first, then build tools. `file`: the file being worked on, for tasks that name it.
pub fn find(root: &Path, file: Option<&Path>, yours: &std::collections::BTreeMap<String, String>) -> Vec<ProjectTask> {
    let mut found = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join(".vscode/tasks.json")) {
        found.extend(vscode_tasks(&text, root, file));
    }
    for (label, command) in yours {
        if !command.trim().is_empty() {
            found.push(ProjectTask { label: label.clone(), command: command.clone(), source: "your tasks" });
        }
    }
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

/// `word` as the shell takes it: as it is when it's plain, in single quotes otherwise.
fn quoted(word: &str) -> String {
    let plain =
        !word.is_empty() && word.chars().all(|c| c.is_alphanumeric() || "-_./=:,@%+".contains(c) || !c.is_ascii());
    if plain { word.to_string() } else { format!("'{}'", word.replace('\'', "'\\''")) }
}

/// `text` with VS Code's `${…}` filled in: the project's folder, the file being worked on.
/// None when it uses one Null can't fill (`${input:…}`, or a file when none is open).
fn filled(text: &str, root: &Path, file: Option<&Path>) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        out.push_str(&rest[..at]);
        let end = rest[at..].find('}')? + at;
        let name = &rest[at + 2..end];
        let file_part = |f: fn(&Path) -> Option<String>| file.and_then(f);
        let value = match name {
            "workspaceFolder" | "workspaceRoot" | "cwd" => Some(root.display().to_string()),
            "workspaceFolderBasename" => root.file_name().map(|n| n.to_string_lossy().into_owned()),
            "pathSeparator" => Some("/".into()),
            "file" => file_part(|f| Some(f.display().to_string())),
            "fileBasename" => file_part(|f| Some(f.file_name()?.to_string_lossy().into_owned())),
            "fileBasenameNoExtension" => file_part(|f| Some(f.file_stem()?.to_string_lossy().into_owned())),
            "fileExtname" => file_part(|f| Some(format!(".{}", f.extension()?.to_string_lossy()))),
            "fileDirname" => file_part(|f| Some(f.parent()?.display().to_string())),
            "relativeFile" => file.and_then(|f| Some(f.strip_prefix(root).ok()?.display().to_string())),
            _ => name.strip_prefix("env:").map(|var| format!("${var}")),
        }?;
        out.push_str(&value);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// The tasks of a `.vscode/tasks.json` (comments and trailing commas allowed, as VS Code
/// has them): each as its label, and the command line it runs, the Mac's variant if it
/// has one. One running in another folder goes there first.
fn vscode_tasks(text: &str, root: &Path, file: Option<&Path>) -> Vec<ProjectTask> {
    use serde_json::Value;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Ok(json) = serde_json::from_str::<Value>(&crate::snippets::without_comments(text)) else {
        return Vec::new();
    };
    let platform = if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    };
    let task_of = |task: &Value| -> Option<ProjectTask> {
        // What this system's variant says, over what the task says.
        let field = |name: &str| task.get(platform).and_then(|p| p.get(name)).or(task.get(name));
        let command = if field("type").and_then(Value::as_str) == Some("npm") {
            format!("npm run {}", field("script")?.as_str()?)
        } else {
            let command = match field("command")? {
                Value::String(c) => c.clone(),
                Value::Object(o) => o.get("value")?.as_str()?.to_string(),
                _ => return None,
            };
            let args: Vec<String> = field("args")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|a| match a {
                    Value::String(s) => Some(quoted(s)),
                    Value::Object(o) => o.get("value")?.as_str().map(quoted),
                    _ => None,
                })
                .collect();
            std::iter::once(command).chain(args).collect::<Vec<_>>().join(" ")
        };
        let mut command = filled(&command, root, file)?;
        if let Some(cwd) = field("options").and_then(|o| o.get("cwd")).and_then(Value::as_str) {
            command = format!("cd {} && {command}", quoted(&filled(cwd, root, file)?));
        }
        let label = task.get("label").and_then(Value::as_str).map(String::from).unwrap_or_else(|| command.clone());
        Some(ProjectTask { label, command, source: ".vscode/tasks.json" })
    };
    json.get("tasks").and_then(Value::as_array).into_iter().flatten().filter_map(task_of).collect()
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
    fn vs_code_tasks_are_read() {
        let root = Path::new("/p/app");
        let file = Path::new("/p/app/src/main.py");
        let text = r#"{
            // Comments and trailing commas, as VS Code has them.
            "version": "2.0.0",
            "tasks": [
                {"label": "build", "type": "shell", "command": "cargo", "args": ["build", "--release"],},
                {"label": "this file", "type": "shell", "command": "python3 ${file}"},
                {"label": "docs", "command": "make html", "options": {"cwd": "${workspaceFolder}/docs site"}},
                {"label": "lint", "type": "npm", "script": "lint"},
                {"label": "mac only", "command": "echo other", "osx": {"command": "echo mac"}},
                {"label": "asks", "command": "deploy ${input:target}"},
                {"label": "quoted", "command": "echo", "args": ["two words", {"value": "it's", "quoting": "strong"}]},
            ],
        }"#;
        let tasks: Vec<(String, String)> =
            vscode_tasks(text, root, Some(file)).into_iter().map(|t| (t.label, t.command)).collect();
        let mac = if cfg!(target_os = "macos") { "echo mac" } else { "echo other" };
        assert_eq!(
            tasks,
            [
                ("build".into(), "cargo build --release".into()),
                ("this file".into(), "python3 /p/app/src/main.py".into()),
                ("docs".into(), "cd '/p/app/docs site' && make html".into()),
                ("lint".into(), "npm run lint".into()),
                ("mac only".into(), mac.into()),
                ("quoted".into(), "echo 'two words' 'it'\\''s'".into()),
            ]
        );
        // With no file open, a task naming one is left out.
        assert_eq!(vscode_tasks(text, root, None).len(), 5);
        assert!(vscode_tasks("not json", root, None).is_empty());
    }

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
        let dir = crate::tools::test_dir("tasks");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package.json"), r#"{"scripts":{"dev":"vite"}}"#).unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(dir.join("Makefile"), "deploy:\n\t./deploy.sh\n").unwrap();
        let commands: Vec<String> = find(&dir, None, &Default::default()).into_iter().map(|t| t.command).collect();
        assert_eq!(commands, ["pnpm run dev", "make deploy"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
