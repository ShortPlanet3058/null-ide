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

/// The environment of the project at `root`: a folder of its with a Python in it, else
/// the one Poetry made for it in its own folder.
pub fn find(root: &Path) -> Option<PathBuf> {
    FOLDERS.iter().map(|f| root.join(f)).find(|env| python_in(env).exists()).or_else(|| poetry(root))
}

/// How the status bar names it: where it is in the project (".venv"), or "Poetry".
pub fn label(env: &Path, root: &Path) -> String {
    match env.strip_prefix(root) {
        Ok(inside) => inside.to_string_lossy().into_owned(),
        Err(_) => "Poetry".into(),
    }
}

/// The environment Poetry keeps for a project outside it (its default): in its
/// `virtualenvs` folder, named after the project and its folder
/// (`name-<hash of the folder>-py3.12`); the newest Python when there are several.
fn poetry(root: &Path) -> Option<PathBuf> {
    poetry_in(root, &poetry_folder()?)
}

/// The same, with Poetry's environments in `folder`.
fn poetry_in(root: &Path, folder: &Path) -> Option<PathBuf> {
    let name = poetry_name(&std::fs::read_to_string(root.join("pyproject.toml")).ok()?)?;
    let prefix = format!("{}-", poetry_env_name(&name, &root.canonicalize().ok()?));
    let mut found: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let version = file.strip_prefix(&prefix)?.strip_prefix("py")?;
            let version = version.split('.').map(|n| n.parse().ok()).collect::<Option<Vec<u32>>>()?;
            Some((version, entry.path()))
        })
        .filter(|(_, env)| python_in(env).exists())
        .collect();
    found.sort();
    found.pop().map(|(_, env)| env)
}

/// The project's name, if Poetry manages it: `[tool.poetry]`'s, or `[project]`'s when
/// `[tool.poetry]` is there too (Poetry 2 reads it from there).
fn poetry_name(pyproject: &str) -> Option<String> {
    let mut section = "";
    let (mut poetry, mut own, mut project) = (false, None, None);
    for line in pyproject.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.split(']').next()) {
            section = header.trim();
            poetry |= section == "tool.poetry" || section.starts_with("tool.poetry.");
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        if key.trim() != "name" {
            continue;
        }
        let value = value.split('#').next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'');
        match section {
            "tool.poetry" => own = Some(value.to_string()),
            "project" => project = Some(value.to_string()),
            _ => {}
        }
    }
    if poetry { own.or(project).filter(|n| !n.is_empty()) } else { None }
}

/// Where Poetry keeps environments: as it's set to (`POETRY_VIRTUALENVS_PATH`, else
/// under `POETRY_CACHE_DIR`), else its cache folder.
fn poetry_folder() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(path) = var("POETRY_VIRTUALENVS_PATH") {
        return Some(path);
    }
    let cache = var("POETRY_CACHE_DIR").or_else(|| {
        let home = var("HOME")?;
        Some(if cfg!(target_os = "macos") {
            home.join("Library/Caches/pypoetry")
        } else {
            var("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache")).join("pypoetry")
        })
    })?;
    Some(cache.join("virtualenvs"))
}

/// Poetry's name for a project's environments, less the Python version: its name made
/// safe (lower case, odd characters as `_`, 42 at most), then the first 8 characters of
/// its folder's SHA-256 in URL-safe base64.
fn poetry_env_name(name: &str, folder: &Path) -> String {
    use base64::Engine;
    use sha2::Digest;
    let safe: String = name
        .to_lowercase()
        .chars()
        .map(|c| if " $`!*@\"\\\r\n\t".contains(c) { '_' } else { c })
        .take(42)
        .collect();
    let hash = sha2::Sha256::digest(folder.to_string_lossy().as_bytes());
    let hash = base64::engine::general_purpose::URL_SAFE.encode(hash);
    format!("{safe}-{}", &hash[..8])
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

    #[test]
    fn poetry_s_own_environment_is_found() {
        // As Poetry names it (worked out with the same code as Poetry, in Python).
        assert_eq!(poetry_env_name("My App", Path::new("/Users/me/my-app")), "my_app-IAp8Z3rM");
        let pyproject = "[project]\nname = \"alpha\"\n\n[tool.poetry]\npackage-mode = false\n";
        assert_eq!(poetry_name(pyproject).as_deref(), Some("alpha"), "Poetry 2: [project]'s name");
        let old = "[tool.poetry]\nname = 'beta'  # the name\n[tool.poetry.dependencies]\nname = \"x\"\n";
        assert_eq!(poetry_name(old).as_deref(), Some("beta"));
        assert_eq!(poetry_name("[project]\nname = \"uv-made\"\n"), None, "not Poetry's");
        // Found among Poetry's: this project's (not another's of the same name), the newest Python.
        let dir = crate::tools::test_dir("poetry-env");
        let _ = std::fs::remove_dir_all(&dir);
        let (project, envs) = (dir.join("alpha"), dir.join("virtualenvs"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("pyproject.toml"), pyproject).unwrap();
        let ours = poetry_env_name("alpha", &project.canonicalize().unwrap());
        for env in [format!("{ours}-py3.9"), format!("{ours}-py3.12"), "alpha-AAAAAAAA-py3.13".into()] {
            std::fs::create_dir_all(envs.join(&env).join("bin")).unwrap();
            std::fs::write(envs.join(&env).join("bin/python"), "").unwrap();
        }
        std::fs::create_dir_all(envs.join(format!("{ours}-py3.14"))).unwrap(); // No Python in it.
        assert_eq!(poetry_in(&project, &envs), Some(envs.join(format!("{ours}-py3.12"))));
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(label(Path::new("/p/.venv"), Path::new("/p")), ".venv");
        assert_eq!(label(Path::new("/cache/virtualenvs/a-x-py3.12"), Path::new("/p")), "Poetry");
    }
}
