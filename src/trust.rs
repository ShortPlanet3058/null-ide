//! Folders you trust to run their own code. A folder someone sent can make programs run by
//! being opened: its `.venv`'s `python` (and `node`) for Python's server, its own
//! TypeScript, a Rust project's build scripts and proc macros, its `.cargo` config and
//! toolchain file. Until it's trusted (Safe mode), no language server starts for it, its
//! terminal doesn't activate its environment, and its `.null/settings.json` only sets how
//! text is edited. Trusted once, remembered (`trusted-folders.json` in Null's data folder).
//! The projects opened before trust was asked for are trusted (the first time, from the
//! recent projects), so nothing changes for those.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

static TRUSTED: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

fn file() -> Option<PathBuf> {
    Some(crate::tools::data_dir()?.join("trusted-folders.json"))
}

/// A folder as kept: one form for each (a link followed, a WSL or server folder plain).
fn kept_form(path: &Path) -> PathBuf {
    let path = crate::remote::plain(crate::wsl::plain(path.to_path_buf()));
    if crate::remote::is_remote(&path) || crate::wsl::is_wsl(&path) {
        return path;
    }
    dunce::canonicalize(&path).unwrap_or(path)
}

fn read(file: &Path) -> Option<Vec<PathBuf>> {
    let text = std::fs::read_to_string(file).ok()?;
    serde_json::from_str(&text).ok()
}

fn with<T>(f: impl FnOnce(&mut Vec<PathBuf>) -> T) -> T {
    let mut trusted = TRUSTED.lock().unwrap_or_else(|e| e.into_inner());
    let list = trusted.get_or_insert_with(|| file().and_then(|f| read(&f)).unwrap_or_default());
    f(list)
}

fn save(list: &[PathBuf]) {
    if let (Some(file), Ok(text)) = (file(), serde_json::to_string_pretty(list)) {
        crate::fs_ops::write_file(&file, text.as_bytes()).ok();
    }
}

/// The first time (no list kept yet): the projects you'd opened before, trusted. Called
/// before any window opens, so the folder this launch opens isn't one of them.
pub fn start() {
    let Some(file) = file() else { return };
    if file.exists() {
        return;
    }
    let list: Vec<PathBuf> = crate::session::recent_projects().iter().map(|p| kept_form(p)).collect();
    save(&list);
    with(|trusted| *trusted = list);
}

/// Whether `path` (a folder, or a file) is in a folder you trust. Null's own folder for
/// when none is open is.
pub fn is_trusted(path: &Path) -> bool {
    // (Tests' folders are trusted, but for those meant not to be.)
    if cfg!(test) {
        return !path.components().any(|c| c.as_os_str() == "untrusted");
    }
    if crate::session::is_scratch(path) {
        return true;
    }
    let path = kept_form(path);
    with(|trusted| trusted.iter().any(|folder| path.starts_with(folder)))
}

/// Trusts `folder` from now on.
pub fn trust(folder: &Path) {
    let folder = kept_form(folder);
    with(|trusted| {
        if !trusted.contains(&folder) {
            trusted.push(folder);
            save(trusted);
        }
    });
}

/// Stops trusting `folder` (and the folders in it trusted on their own).
pub fn distrust(folder: &Path) {
    let folder = kept_form(folder);
    with(|trusted| {
        let before = trusted.len();
        trusted.retain(|f| !f.starts_with(&folder));
        if trusted.len() != before {
            save(trusted);
        }
    });
}
