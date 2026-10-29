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
    let list = seeded(crate::session::recent_projects(), crate::tools::home_dir().as_deref());
    save(&list);
    with(|trusted| *trusted = list);
}

/// The recent projects trusted the first time: not your home folder or a disk's root (a
/// folder trusted is trusted with all that's in it: opened once, that would be everything).
fn seeded(recent: Vec<PathBuf>, home: Option<&Path>) -> Vec<PathBuf> {
    let home = home.map(kept_form);
    recent.iter().map(|p| kept_form(p)).filter(|p| p.parent().is_some() && Some(p) != home.as_ref()).collect()
}

/// Whether `path` (a folder, or a file) is in a folder you trust. Null's own folders are:
/// its data (the Scratch folder for when none is open, What's New) and your settings'.
pub fn is_trusted(path: &Path) -> bool {
    // (Tests' folders are trusted, but for those meant not to be.)
    if cfg!(test) {
        return !path.components().any(|c| c.as_os_str() == "untrusted");
    }
    let path = kept_form(path);
    let settings = crate::settings::Settings::path().and_then(|p| p.parent().map(Path::to_path_buf));
    if [crate::tools::data_dir(), settings].iter().flatten().any(|own| path.starts_with(kept_form(own))) {
        return true;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_time_recent_projects_are_trusted_but_not_home() {
        let home = crate::tools::test_dir("trust-home");
        std::fs::create_dir_all(home.join("app")).unwrap();
        let recent = vec![home.join("app"), home.clone(), PathBuf::from("/"), PathBuf::from("/ssh:box:/srv/app")];
        let trusted = seeded(recent, Some(&home));
        assert_eq!(trusted, [kept_form(&home.join("app")), PathBuf::from("/ssh:box:/srv/app")]);
        std::fs::remove_dir_all(&home).ok();
    }
}
