//! A project's `.vscode/launch.json`: how its program is debugged (what runs, with which
//! arguments, where, with what in its environment), for the debuggers Null runs (lldb:
//! CodeLLDB's, the C/C++ extension's, lldb-dap's own).

use serde_json::Value;
use std::path::{Path, PathBuf};

/// One way of debugging the program, as launch.json says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub name: String,
    /// The program; None when cargo builds it (`cargo` below) and says where it went.
    pub program: Option<PathBuf>,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Added to the program's environment.
    pub env: Vec<(String, String)>,
    /// CodeLLDB's `"cargo": { "args": [...] }`: what to build it with.
    pub cargo: Option<Vec<String>>,
}

/// The debuggers whose configurations Null can run (they all mean lldb or gdb on a native
/// program).
const NATIVE: &[&str] = &["lldb", "lldb-dap", "lldb-vscode", "cppdbg", "cppvsdbg", "codelldb"];

/// The configurations of `root`'s `.vscode/launch.json` Null can run, in its order.
pub fn read(root: &Path, file: Option<&Path>) -> Vec<Launch> {
    let Ok(text) = std::fs::read_to_string(root.join(".vscode/launch.json")) else { return Vec::new() };
    parse(&text, root, file)
}

fn parse(text: &str, root: &Path, file: Option<&Path>) -> Vec<Launch> {
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
    let fill = |s: &str| crate::tasks::filled(s, root, file, str::to_string);
    json["configurations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|configuration| {
            // This Mac's own settings over the shared ones.
            let mut c = configuration.as_object()?.clone();
            if let Some(own) = configuration[platform].as_object() {
                c.extend(own.clone());
            }
            let kind = c.get("type")?.as_str()?;
            if !NATIVE.contains(&kind) || c.get("request").and_then(Value::as_str) != Some("launch") {
                return None;
            }
            let strings = |v: Option<&Value>| -> Option<Vec<String>> {
                v.and_then(Value::as_array)
                    .map_or(Some(Vec::new()), |list| list.iter().map(|a| a.as_str().and_then(fill)).collect())
            };
            let cargo = match c.get("cargo") {
                Some(cargo) => Some(strings(cargo.get("args"))?),
                None => None,
            };
            let program = match c.get("program").and_then(Value::as_str) {
                Some(program) => Some(absolute(root, &fill(program)?)),
                None if cargo.is_some() => None,
                None => return None,
            };
            let cwd = match c.get("cwd").and_then(Value::as_str) {
                Some(cwd) => absolute(root, &fill(cwd)?),
                None => root.to_path_buf(),
            };
            // `{"K": "v"}`, or the C/C++ extension's `[{"name": "K", "value": "v"}]`.
            let env = match c.get("env").or(c.get("environment")) {
                Some(Value::Object(map)) => {
                    map.iter().filter_map(|(k, v)| Some((k.clone(), fill(v.as_str()?)?))).collect()
                }
                Some(Value::Array(list)) => list
                    .iter()
                    .filter_map(|e| Some((e["name"].as_str()?.to_string(), fill(e["value"].as_str()?)?)))
                    .collect(),
                _ => Vec::new(),
            };
            let name = c.get("name").and_then(Value::as_str).unwrap_or("launch.json").to_string();
            Some(Launch { name, program, args: strings(c.get("args"))?, cwd, env, cargo })
        })
        .collect()
}

/// `path` as written, from the project's folder when it's relative.
fn absolute(root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() { path } else { root.join(path) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_configurations_null_can_run() {
        let root = Path::new("/p");
        let text = r#"{
            // CodeLLDB's, built by cargo.
            "version": "0.2.0",
            "configurations": [
                { "type": "debugpy", "request": "launch", "name": "Python", "program": "${file}" },
                {
                    "type": "lldb", "request": "launch", "name": "Debug app",
                    "cargo": { "args": ["build", "--bin=app"] },
                    "args": ["--port", "8080"],
                    "cwd": "${workspaceFolder}/run",
                    "env": { "RUST_LOG": "debug" },
                },
                { "type": "lldb", "request": "attach", "name": "Attach", "pid": 1 },
                {
                    "type": "cppdbg", "request": "launch", "name": "C",
                    "program": "build/tool", "args": [],
                    "environment": [{ "name": "MODE", "value": "${workspaceFolderBasename}" }],
                    "osx": { "program": "build/tool-mac" },
                },
                { "type": "lldb", "request": "launch", "name": "Asks", "program": "${input:which}" },
            ],
        }"#;
        let launches = parse(text, root, None);
        assert_eq!(launches.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Debug app", "C"]);
        let app = &launches[0];
        assert_eq!(app.program, None);
        assert_eq!(app.cargo.as_deref(), Some(&["build".to_string(), "--bin=app".to_string()][..]));
        assert_eq!(app.args, ["--port", "8080"]);
        assert_eq!(app.cwd, Path::new("/p/run"));
        assert_eq!(app.env, [("RUST_LOG".to_string(), "debug".to_string())]);
        let c = &launches[1];
        let mac = if cfg!(target_os = "macos") { "/p/build/tool-mac" } else { "/p/build/tool" };
        assert_eq!(c.program.as_deref(), Some(Path::new(mac)));
        assert_eq!((c.cwd.as_path(), &c.env[..]), (root, &[("MODE".to_string(), "p".to_string())][..]));
        assert!(parse("not json", root, None).is_empty());
    }
}
