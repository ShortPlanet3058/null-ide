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

/// `text` with VS Code's `${…}` filled in: the project's folder, the file being worked on,
/// a variable of the environment; each value put in by `put` (quoted for the shell, or as
/// it is). None when it uses one Null can't fill (`${input:…}`, or a file when none is
/// open).
fn filled(text: &str, root: &Path, file: Option<&Path>, put: fn(&str) -> String) -> Option<String> {
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
            // As VS Code: its value now (none set, nothing).
            _ => name.strip_prefix("env:").map(|var| std::env::var(var).unwrap_or_default()),
        }?;
        out.push_str(&put(&value));
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// A part of a shell task's command line, as VS Code writes it: in quotes when it holds a
/// space or a quote, else as written (so `*.rs`, `2>&1`, `$HOME` keep their meaning). A
/// filled-in value is quoted in its own right, so a file's name is never taken as shell.
fn shell_part(part: &str, root: &Path, file: Option<&Path>) -> Option<String> {
    if part.contains(|c: char| c.is_whitespace() || c == '\'' || c == '"') {
        Some(quoted(&filled(part, root, file, str::to_string)?))
    } else {
        filled(part, root, file, quoted)
    }
}

/// The tasks of a `.vscode/tasks.json` (comments and trailing commas allowed, as VS Code
/// has them): each as its label, and the command line it runs, the Mac's variant if it
/// has one. One running in another folder runs there (in a subshell, so the terminal
/// stays where it is); one depending on others runs them first, in order.
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
    let string = |v: &Value| match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("value")?.as_str().map(String::from),
        _ => None,
    };
    // Each task's own command (None: it only runs others, or can't be filled in), and what
    // it depends on.
    let own = |task: &Value| -> (Option<String>, Vec<String>) {
        // What this system's variant says, over what the task says.
        let field = |name: &str| task.get(platform).and_then(|p| p.get(name)).or(task.get(name));
        let depends: Vec<String> = match field("dependsOn") {
            Some(Value::String(label)) => vec![label.clone()],
            Some(Value::Array(labels)) => labels.iter().filter_map(|l| l.as_str().map(String::from)).collect(),
            _ => Vec::new(),
        };
        let command = (|| {
            let kind = field("type").and_then(Value::as_str).unwrap_or("shell");
            let (command, cwd) = if kind == "npm" {
                let path = field("path").and_then(Value::as_str).filter(|p| !p.is_empty());
                let runner = package_runner(&path.map_or(root.to_path_buf(), |p| root.join(p)));
                (format!("{runner} run {}", quoted(field("script")?.as_str()?)), path.map(|p| root.join(p)))
            } else {
                let command = string(field("command")?)?;
                let args: Vec<String> =
                    field("args").and_then(Value::as_array).into_iter().flatten().filter_map(string).collect();
                let line = if kind == "process" {
                    // One program and its arguments: each part exactly as written.
                    let mut parts = vec![quoted(&filled(&command, root, file, str::to_string)?)];
                    for arg in &args {
                        parts.push(quoted(&filled(arg, root, file, str::to_string)?));
                    }
                    parts.join(" ")
                } else if args.is_empty() {
                    // A whole command line.
                    filled(&command, root, file, quoted)?
                } else {
                    let mut parts = vec![shell_part(&command, root, file)?];
                    for arg in &args {
                        parts.push(shell_part(arg, root, file)?);
                    }
                    parts.join(" ")
                };
                let cwd = field("options").and_then(|o| o.get("cwd")).and_then(Value::as_str);
                let cwd = match cwd {
                    Some(cwd) => Some(std::path::PathBuf::from(filled(cwd, root, file, str::to_string)?)),
                    None => None,
                };
                (line, cwd)
            };
            Some(match cwd {
                Some(dir) => format!("(cd {} && {command})", quoted(&dir.display().to_string())),
                None => command,
            })
        })();
        (command, depends)
    };
    let tasks: Vec<(String, Option<String>, Vec<String>)> = json
        .get("tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|task| {
            let (command, depends) = own(task);
            let label = task.get("label").and_then(Value::as_str).map(String::from);
            (label.or(command.clone()).unwrap_or_default(), command, depends)
        })
        .collect();
    // A task's whole run: what it depends on first, then itself. None when one of them
    // can't run (missing, can't be filled in, or depending on itself).
    fn whole(
        label: &str,
        tasks: &[(String, Option<String>, Vec<String>)],
        seen: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if seen.iter().any(|s| s == label) {
            return None;
        }
        seen.push(label.to_string());
        let (_, command, depends) = tasks.iter().find(|(l, ..)| l == label)?;
        let mut run = Vec::new();
        for dependency in depends {
            run.extend(whole(dependency, tasks, seen)?);
        }
        if let Some(command) = command {
            run.push(command.clone());
        } else if depends.is_empty() {
            return None;
        }
        seen.pop();
        Some(run)
    }
    tasks
        .iter()
        .filter(|(label, ..)| !label.is_empty())
        .filter_map(|(label, ..)| {
            let run = whole(label, &tasks, &mut Vec::new())?;
            Some(ProjectTask { label: label.clone(), command: run.join(" && "), source: ".vscode/tasks.json" })
        })
        .collect()
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
                {"label": "shell", "command": "ls", "args": ["*.rs", "2>&1", "${fileBasename}"]},
                {"label": "exact", "type": "process", "command": "${workspaceFolder}/run.sh", "args": ["*.rs"]},
                {"label": "web", "type": "npm", "script": "dev", "path": "web/"},
                {"label": "all", "dependsOn": ["build", "lint"]},
                {"label": "loop", "command": "x", "dependsOn": "loop"},
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
                ("docs".into(), "(cd '/p/app/docs site' && make html)".into()),
                ("lint".into(), "npm run lint".into()),
                ("mac only".into(), mac.into()),
                ("quoted".into(), "echo 'two words' 'it'\\''s'".into()),
                // Shell syntax kept as written; a filled-in name quoted only as it needs.
                ("shell".into(), "ls *.rs 2>&1 main.py".into()),
                ("exact".into(), "/p/app/run.sh '*.rs'".into()),
                ("web".into(), "(cd /p/app/web/ && npm run dev)".into()),
                ("all".into(), "cargo build --release && npm run lint".into()),
            ]
        );
        // With no file open, a task naming one is left out.
        assert_eq!(vscode_tasks(text, root, None).len(), 8);
        // A file's name is never taken as shell.
        let named = r#"{"tasks": [{"label": "run", "command": "python3", "args": ["${file}"]}]}"#;
        let odd = Path::new("/p/app/x';rm -rf ~;'.py");
        assert_eq!(vscode_tasks(named, root, Some(odd))[0].command, "python3 '/p/app/x'\\'';rm -rf ~;'\\''.py'");
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
