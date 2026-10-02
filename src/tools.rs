//! Finding the command line tools Null runs (language servers, Node, the AI tools):
//! on the PATH, then where their installers usually put them, since an app opened
//! from the Finder doesn't get the terminal's PATH.

use std::path::PathBuf;

/// Where Null keeps what it installs itself (language servers), never on the system:
/// `~/Library/Application Support/Null` on macOS, `$XDG_DATA_HOME/null` (or
/// `~/.local/share/null`) on Linux, `%LOCALAPPDATA%\Null` on Windows.
pub fn data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        Some(home?.join("Library/Application Support/Null"))
    } else if cfg!(windows) {
        Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Null"))
    } else if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(PathBuf::from(xdg).join("null"))
    } else {
        Some(home?.join(".local/share/null"))
    }
}

/// Folders a tool may be in, most likely first.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for dir in
            [".local/bin", ".cargo/bin", "go/bin", ".claude/local", ".npm-global/bin", ".bun/bin", ".volta/bin", "bin"]
        {
            dirs.push(home.join(dir));
        }
        // Node versions installed with nvm.
        if let Ok(versions) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            dirs.extend(versions.flatten().map(|v| v.path().join("bin")));
        }
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/local/go/bin"].map(PathBuf::from));
    // What Null installed itself.
    if let Some(data) = data_dir() {
        dirs.push(data.join("servers/node_modules/.bin"));
        dirs.push(data.join("servers/bin"));
    }
    dirs
}

/// A PATH for the tools Null starts, so they find what they need in turn (node for npm packages).
pub fn search_path() -> std::ffi::OsString {
    std::env::join_paths(search_dirs()).unwrap_or_default()
}

/// Where `name` is installed, if it is.
pub fn find(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd"), name.to_string()]
    } else {
        vec![name.to_string()]
    };
    search_dirs()
        .into_iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)).collect::<Vec<_>>())
        .find(|p| p.is_file())
}
