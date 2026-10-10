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
    if let Some(found) = MEMO.with(|memo| memo.borrow().as_ref().and_then(|m| m.get(root).cloned())) {
        return found;
    }
    let found = FOLDERS.iter().map(|f| root.join(f)).find(|env| python_in(env).exists()).or_else(|| poetry(root));
    MEMO.with(|memo| {
        if let Some(memo) = memo.borrow_mut().as_mut() {
            memo.insert(root.to_path_buf(), found.clone());
        }
    });
    found
}

thread_local! {
    /// Environments already looked for, while `remembering` (a whole project's tests found
    /// at once: each folder looked in once, not once a test).
    static MEMO: std::cell::RefCell<Option<std::collections::HashMap<PathBuf, Option<PathBuf>>>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `work` with each folder's environment looked for once.
pub fn remembering<T>(work: impl FnOnce() -> T) -> T {
    MEMO.with(|memo| *memo.borrow_mut() = Some(Default::default()));
    let result = work();
    MEMO.with(|memo| *memo.borrow_mut() = None);
    result
}

/// The environment for `path` in the project at `root`: the nearest folder's, from
/// `path`'s up to the project's (each package of a monorepo with its own), else the
/// project's.
pub fn find_for(root: &Path, path: &Path) -> Option<PathBuf> {
    owner_for(root, path).map(|(_, env)| env)
}

/// The same, with the folder it's for (the package's, or the project's).
pub fn owner_for(root: &Path, path: &Path) -> Option<(PathBuf, PathBuf)> {
    path.ancestors()
        .take_while(|dir| dir.starts_with(root) && *dir != root)
        .chain(std::iter::once(root))
        .find_map(|dir| find(dir).map(|env| (dir.to_path_buf(), env)))
}

/// Whether a change at `path` (in the project at `root`) can make or remove an
/// environment: its folder, Poetry's files.
pub fn may_change(root: &Path, path: &Path) -> bool {
    let Ok(inside) = path.strip_prefix(root) else { return false };
    inside.components().any(|c| FOLDERS.iter().any(|f| c.as_os_str() == *f))
        || path.file_name().is_some_and(|n| ["pyproject.toml", "poetry.lock", "poetry.toml"].iter().any(|f| n == *f))
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
/// (`name-<hash of the folder>-py3.12`).
fn poetry(root: &Path) -> Option<PathBuf> {
    let pyproject = std::fs::read_to_string(root.join("pyproject.toml")).ok()?;
    let local = std::fs::read_to_string(root.join("poetry.toml")).unwrap_or_default();
    let name = poetry_name(&pyproject, root.join("poetry.lock").exists())?;
    poetry_in(root, &name, &poetry_folder(&local)?)
}

/// The same, with Poetry's environments in `folder`: the one `poetry env use` chose
/// (`envs.toml` says), else the newest Python.
fn poetry_in(root: &Path, name: &str, folder: &Path) -> Option<PathBuf> {
    let base = poetry_env_name(name, &root.canonicalize().ok()?);
    let prefix = format!("{base}-py");
    let mut found: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let version = file.strip_prefix(&prefix)?;
            let version = version.split('.').map(|n| n.parse().ok()).collect::<Option<Vec<u32>>>()?;
            Some((version, entry.path()))
        })
        .filter(|(_, env)| python_in(env).exists())
        .collect();
    let chosen = std::fs::read_to_string(folder.join("envs.toml")).ok().and_then(|t| toml_value(&t, &base, "minor"));
    if let Some(env) = chosen.and_then(|minor| found.iter().find(|(_, env)| env.ends_with(format!("{prefix}{minor}")))) {
        return Some(env.1.clone());
    }
    found.sort();
    found.pop().map(|(_, env)| env)
}

/// The project's name as Poetry names its environment, if Poetry manages it (it has
/// `[tool.poetry]`, Poetry's lock file, or Poetry builds it): `[project]`'s, else
/// `[tool.poetry]`'s, else Poetry's own for a project that isn't a package.
fn poetry_name(pyproject: &str, locked: bool) -> Option<String> {
    let has_poetry = pyproject.lines().map(str::trim).any(|l| l == "[tool.poetry]" || l.starts_with("[tool.poetry."));
    let built = toml_value(pyproject, "build-system", "build-backend").is_some_and(|b| b.starts_with("poetry"));
    if !(has_poetry || locked || built) {
        return None;
    }
    let name = toml_value(pyproject, "project", "name")
        .or_else(|| toml_value(pyproject, "tool.poetry", "name"))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "non-package-mode".into());
    // As Python packaging writes names: "My_App.core" is "my-app-core".
    let mut canonical = String::new();
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            if !canonical.ends_with('-') {
                canonical.push('-');
            }
        } else {
            canonical.extend(c.to_lowercase());
        }
    }
    Some(canonical)
}

/// A plain value from TOML text: `key = "value"` under `[section]`, or `section.key =
/// "value"` before any. (Enough for the few settings read here, not a TOML reader.)
fn toml_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            current = line.trim_start_matches('[').split(']').next().unwrap_or("").trim().trim_matches('"').to_string();
            continue;
        }
        let Some((k, value)) = line.split_once('=') else { continue };
        let k = k.trim().trim_matches('"');
        let full = if current.is_empty() { k.to_string() } else { format!("{current}.{k}") };
        if full == format!("{section}.{key}") || (section.is_empty() && full == key) {
            let value = value.trim();
            let value = match value.chars().next() {
                Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or(""),
                _ => value.split('#').next().unwrap_or("").trim(),
            };
            return Some(value.to_string());
        }
    }
    None
}

/// Where Poetry keeps environments: as it's set to (`POETRY_VIRTUALENVS_PATH`; the
/// project's `poetry.toml`, `local`; Poetry's own `config.toml`), else its cache folder.
fn poetry_folder(local: &str) -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = var("HOME");
    let config_dir = var("POETRY_CONFIG_DIR").or_else(|| {
        let home = home.clone()?;
        Some(if cfg!(target_os = "macos") {
            home.join("Library/Application Support/pypoetry")
        } else {
            var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")).join("pypoetry")
        })
    });
    let config = config_dir.and_then(|d| std::fs::read_to_string(d.join("config.toml")).ok()).unwrap_or_default();
    let setting = |key: &str| {
        let (section, key) = key.rsplit_once('.').unwrap_or(("", key));
        toml_value(local, section, key).or_else(|| toml_value(&config, section, key))
    };
    let expand = |path: String| match (path.strip_prefix("~/"), &home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    };
    let cache = var("POETRY_CACHE_DIR").or_else(|| setting("cache-dir").map(expand)).or_else(|| {
        let home = home.clone()?;
        Some(if cfg!(target_os = "macos") {
            home.join("Library/Caches/pypoetry")
        } else {
            var("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache")).join("pypoetry")
        })
    })?;
    if let Some(path) = var("POETRY_VIRTUALENVS_PATH") {
        return Some(path);
    }
    match setting("virtualenvs.path") {
        Some(path) => Some(expand(path.replace("{cache-dir}", &cache.to_string_lossy()))),
        None => Some(cache.join("virtualenvs")),
    }
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
#[cfg(test)]
pub fn python(root: &Path) -> String {
    python_for(root, root)
}

/// The same, for what's at `path` (its nearest environment: see `find_for`).
pub fn python_for(root: &Path, path: &Path) -> String {
    match find_for(root, path) {
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

    /// A monorepo: each package's files use its environment, the rest the project's.
    #[test]
    fn the_nearest_environment_is_used() {
        let dir = crate::tools::test_dir("python-env-nested");
        let _ = std::fs::remove_dir_all(&dir);
        for env in [".venv", "api/.venv"] {
            std::fs::create_dir_all(dir.join(env).join("bin")).unwrap();
            std::fs::write(dir.join(env).join("bin/python"), "").unwrap();
        }
        std::fs::create_dir_all(dir.join("api/tests")).unwrap();
        std::fs::create_dir_all(dir.join("web")).unwrap();
        assert_eq!(find_for(&dir, &dir.join("api/tests/test_a.py")), Some(dir.join("api/.venv")));
        assert_eq!(python_for(&dir, &dir.join("api/tests/test_a.py")), "api/.venv/bin/python");
        assert_eq!(find_for(&dir, &dir.join("web/x.py")), Some(dir.join(".venv")));
        assert_eq!(find_for(&dir, &dir), Some(dir.join(".venv")));
        assert_eq!(find_for(&dir, Path::new("/elsewhere/x.py")), Some(dir.join(".venv")), "outside: the project's");
        assert!(may_change(&dir, &dir.join("api/.venv/bin")) && may_change(&dir, &dir.join("api/pyproject.toml")));
        assert!(!may_change(&dir, &dir.join("api/app.py")) && !may_change(&dir, &dir.join("environment.py")));
        assert!(!may_change(&dir.join("api"), &dir.join("api/app.py")), "a folder above the project isn't one");
        assert_eq!(owner_for(&dir, &dir.join("api/tests/test_a.py")), Some((dir.join("api"), dir.join("api/.venv"))));
        let memo = remembering(|| find_for(&dir, &dir.join("web/x.py")));
        assert_eq!(memo, Some(dir.join(".venv")), "remembered: the same");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn poetry_s_own_environment_is_found() {
        // As Poetry names it (worked out with the same code as Poetry, in Python).
        assert_eq!(poetry_env_name("My App", Path::new("/Users/me/my-app")), "my_app-IAp8Z3rM");
        let pyproject = "[project]\nname = \"alpha\"\n\n[tool.poetry]\npackage-mode = false\n";
        assert_eq!(poetry_name(pyproject, false).as_deref(), Some("alpha"), "Poetry 2: [project]'s name");
        let old = "[tool.poetry]\nname = 'beta'  # the name\n[[tool.poetry.source]]\nname = \"x\"\n";
        assert_eq!(poetry_name(old, false).as_deref(), Some("beta"));
        let both = "[project]\nname = \"new\"\n[tool.poetry]\nname = \"old\"\n";
        assert_eq!(poetry_name(both, false).as_deref(), Some("new"), "[project]'s wins, as in Poetry");
        assert_eq!(poetry_name("[project]\nname = \"uv-made\"\n", false), None, "not Poetry's");
        assert_eq!(poetry_name("[project]\nname = \"My_App.core\"\n", true).as_deref(), Some("my-app-core"), "locked: Poetry's");
        let built = "[project]\nname = \"x\"\n[build-system]\nbuild-backend = \"poetry.core.masonry.api\"\n";
        assert_eq!(poetry_name(built, false).as_deref(), Some("x"), "built by Poetry");
        assert_eq!(poetry_name("[tool.poetry]\npackage-mode = false\n", false).as_deref(), Some("non-package-mode"));
        assert_eq!(toml_value("virtualenvs.path = \"/v\" # set\n", "virtualenvs", "path").as_deref(), Some("/v"));
        assert_eq!(toml_value("cache-dir = '/c'\n[virtualenvs]\npath = \"{cache-dir}/v\"\n", "", "cache-dir").as_deref(), Some("/c"));
        assert_eq!(toml_value("[virtualenvs]\npath = \"{cache-dir}/v\"\n", "virtualenvs", "path").as_deref(), Some("{cache-dir}/v"));
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
        assert_eq!(poetry_in(&project, "alpha", &envs), Some(envs.join(format!("{ours}-py3.12"))));
        // The one `poetry env use 3.9` chose.
        std::fs::write(envs.join("envs.toml"), format!("[{ours}]\nminor = \"3.9\"\npatch = \"3.9.18\"\n")).unwrap();
        assert_eq!(poetry_in(&project, "alpha", &envs), Some(envs.join(format!("{ours}-py3.9"))));
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(label(Path::new("/p/.venv"), Path::new("/p")), ".venv");
        assert_eq!(label(Path::new("/p/api/.venv"), Path::new("/p")), "api/.venv");
        assert_eq!(label(Path::new("/cache/virtualenvs/a-x-py3.12"), Path::new("/p")), "Poetry");
    }
}
