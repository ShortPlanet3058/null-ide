//! A Python project's own environment (`.venv`, made by `python -m venv`, uv or Poetry):
//! what its tests run with, what its language server reads its packages from, what a
//! terminal opened in it has on its PATH. As VS Code does, without being told.

use std::path::{Path, PathBuf};

/// Where projects keep their environment, in the order looked at. (Not `.env`: that's
/// where many keep their secrets as `NAME=value` lines.)
const FOLDERS: [&str; 3] = [".venv", "venv", "env"];

/// The environment's own Python, inside it.
fn python_in(env: &Path) -> PathBuf {
    if cfg!(windows) { env.join("Scripts").join("python.exe") } else { env.join("bin").join("python") }
}

/// The environment of the project at `root`: a folder of its with a Python in it.
pub fn find(root: &Path) -> Option<PathBuf> {
    FOLDERS.iter().map(|f| root.join(f)).find(|env| python_in(env).exists())
}

/// What runs Python for the project's commands, as typed from its folder: its
/// environment's (`.venv/bin/python`), else the system's `python3`.
pub fn python(root: &Path) -> String {
    match find(root) {
        Some(env) => {
            let python = python_in(&env);
            let relative = python.strip_prefix(root).unwrap_or(&python).to_string_lossy().into_owned();
            if relative.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')) {
                relative
            } else {
                format!("'{}'", relative.replace('\'', r"'\''"))
            }
        }
        None => "python3".into(),
    }
}

/// What a program started in the project gets, for its environment to come first: its
/// `bin` at the front of PATH, and `VIRTUAL_ENV` (what activating it sets).
pub fn variables(root: &Path) -> Vec<(String, String)> {
    let Some(env) = find(root) else { return Vec::new() };
    let bin = python_in(&env).parent().map(Path::to_path_buf).unwrap_or_default();
    let mut vars = vec![("VIRTUAL_ENV".to_string(), env.to_string_lossy().into_owned())];
    // (A folder whose name has a ":" can't go into PATH: left as it was, not emptied.)
    let path = std::env::var_os("PATH").filter(|p| !p.is_empty());
    let others = path.as_deref().map(|p| std::env::split_paths(p).collect::<Vec<_>>()).unwrap_or_default();
    if let Ok(joined) = std::env::join_paths(std::iter::once(bin).chain(others)) {
        vars.insert(0, ("PATH".into(), joined.to_string_lossy().into_owned()));
    }
    vars
}

/// What a terminal's shell types to make the environment its own: its `activate`, for
/// that shell, from where it is. None for a shell it doesn't know.
pub fn activate_command(env: &Path, shell: &str) -> Option<String> {
    let name = Path::new(shell).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let script = |file: &str| {
        let path = env.join("bin").join(file).to_string_lossy().into_owned();
        format!("'{}'", path.replace('\'', r"'\''"))
    };
    // A space first: kept out of the shell's history (fish; zsh and bash when set to).
    Some(match name.as_str() {
        "zsh" | "bash" | "ksh" | "mksh" => format!(" source {}", script("activate")),
        "sh" | "dash" => format!(" . {}", script("activate")),
        "fish" => format!(" source {}", script("activate.fish")),
        "csh" | "tcsh" => format!(" source {}", script("activate.csh")),
        "nu" => format!(" overlay use {}", script("activate.nu")),
        "pwsh" => format!(" & {}", script("Activate.ps1")),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_s_environment_is_found() {
        let dir = crate::tools::test_dir("python-env");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(find(&dir), None);
        assert_eq!(python(&dir), "python3");
        assert!(variables(&dir).is_empty());
        // A folder called venv with no Python in it isn't one.
        std::fs::create_dir_all(dir.join("venv")).unwrap();
        assert_eq!(find(&dir), None);
        std::fs::create_dir_all(dir.join(".venv/bin")).unwrap();
        std::fs::write(dir.join(".venv/bin/python"), "").unwrap();
        assert_eq!(find(&dir), Some(dir.join(".venv")));
        assert_eq!(python(&dir), ".venv/bin/python");
        let vars = variables(&dir);
        assert!(vars[0].1.starts_with(&dir.join(".venv/bin").to_string_lossy().into_owned()));
        assert_eq!(vars[1], ("VIRTUAL_ENV".to_string(), dir.join(".venv").to_string_lossy().into_owned()));
        let activate = |env: &str, shell: &str| activate_command(Path::new(env), shell);
        assert_eq!(activate("/p/.venv", "/bin/zsh").as_deref(), Some(" source '/p/.venv/bin/activate'"));
        assert_eq!(activate(".venv", "/bin/zsh").as_deref(), Some(" source '.venv/bin/activate'"));
        assert_eq!(activate("/p/.venv", "/opt/homebrew/bin/fish").as_deref(), Some(" source '/p/.venv/bin/activate.fish'"));
        assert_eq!(activate(".venv", "/bin/sh").as_deref(), Some(" . '.venv/bin/activate'"));
        assert_eq!(activate(".venv", "/usr/local/bin/nu").as_deref(), Some(" overlay use '.venv/bin/activate.nu'"));
        assert_eq!(activate(".venv", "/usr/bin/xonsh"), None, "one it doesn't know: nothing typed");
        std::fs::remove_dir_all(&dir).ok();
    }
}
