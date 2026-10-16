//! Folders inside WSL, on Windows: the project is at `\\wsl.localhost\<distro>\home\me\proj`
//! (Windows reads and writes its files there as any others), and every program Null runs
//! for it (the terminal, git, language servers) runs inside Linux, through `wsl.exe`, in the
//! folder's Linux path (`/home/me/proj`). Paths go between the two forms here.

use std::path::{Path, PathBuf};

/// A path inside a WSL distribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WslPath {
    pub distro: String,
    /// As Linux has it: "/home/me/proj".
    pub linux: String,
}

/// The prefixes Windows gives WSL's files.
const HOSTS: [&str; 2] = ["wsl.localhost", "wsl$"];

/// `path` as a WSL path, when it's one (`\\wsl.localhost\Ubuntu\home\me`, `\\wsl$\Ubuntu\…`,
/// and the `\\?\UNC\…` form); separators either way.
pub fn of(path: &Path) -> Option<WslPath> {
    let text = path.to_str()?.replace('/', "\\");
    let rest = text.strip_prefix(r"\\?\UNC\").or_else(|| text.strip_prefix(r"\\"))?;
    let (host, rest) = rest.split_once('\\')?;
    if !HOSTS.iter().any(|h| host.eq_ignore_ascii_case(h)) {
        return None;
    }
    let (distro, inside) = rest.split_once('\\').unwrap_or((rest, ""));
    if distro.is_empty() {
        return None;
    }
    let linux = format!("/{}", inside.trim_matches('\\').replace('\\', "/"));
    Some(WslPath { distro: distro.to_string(), linux })
}

/// Whether `path` is inside WSL.
pub fn is_wsl(path: &Path) -> bool {
    of(path).is_some()
}

/// A Linux path of `distro` as Windows reaches it.
pub fn windows_path(distro: &str, linux: &str) -> PathBuf {
    let inside = linux.trim_start_matches('/').replace('/', "\\");
    if inside.is_empty() {
        PathBuf::from(format!(r"\\wsl.localhost\{distro}"))
    } else {
        PathBuf::from(format!(r"\\wsl.localhost\{distro}\{inside}"))
    }
}

/// `path` as the programs running in `distro` know it: its Linux path when it's in there.
/// (A Windows path, `C:\x`, is `/mnt/c/x` there.)
pub fn linux_path(distro: &str, path: &Path) -> String {
    if let Some(wsl) = of(path).filter(|w| w.distro.eq_ignore_ascii_case(distro)) {
        return wsl.linux;
    }
    let text = path.to_string_lossy().replace('\\', "/");
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {
            format!("/mnt/{}{}", drive.to_ascii_lowercase(), &text[2..])
        }
        _ => text,
    }
}

/// A path a program in `distro` gave (`/home/me/proj/src/a.rs`, `/mnt/c/x`) as Windows
/// reaches it.
pub fn from_linux(distro: &str, linux: &str) -> PathBuf {
    let rest = linux.strip_prefix("/mnt/");
    if let Some(rest) = rest {
        let mut parts = rest.splitn(2, '/');
        if let Some(drive) = parts.next().filter(|d| d.len() == 1 && d.chars().all(|c| c.is_ascii_alphabetic())) {
            let tail = parts.next().unwrap_or("").replace('/', "\\");
            return PathBuf::from(format!("{}:\\{tail}", drive.to_ascii_uppercase()));
        }
    }
    windows_path(distro, linux)
}

/// The distributions installed, by name (none off Windows, or without WSL).
pub fn distros() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let Ok(output) = crate::tools::command("wsl.exe").args(["--list", "--quiet"]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    names_listed(&output.stdout)
}

/// wsl.exe --list --quiet's output: UTF-16 (little end first), a name a line.
fn names_listed(bytes: &[u8]) -> Vec<String> {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    let text = String::from_utf16_lossy(&units);
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The home folder in `distro` (a Linux path), asked once.
pub fn home(distro: &str) -> Option<String> {
    use std::sync::Mutex;
    static HOMES: Mutex<Vec<(String, Option<String>)>> = Mutex::new(Vec::new());
    if let Some((_, home)) = HOMES.lock().ok()?.iter().find(|(d, _)| d == distro) {
        return home.clone();
    }
    let home = command(distro, None, "printenv", &["HOME"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| h.starts_with('/'));
    HOMES.lock().ok()?.push((distro.to_string(), home.clone()));
    home
}

/// The folder a shell in `distro` says it's in, in its title (bash's `me@host: ~/proj`), as
/// Windows reaches it.
pub fn folder_in_title(distro: &str, title: &str) -> Option<PathBuf> {
    let shown = title.rsplit_once(": ").map_or(title, |(_, f)| f).trim();
    let linux = match shown.strip_prefix('~') {
        Some(rest) => format!("{}{rest}", home(distro)?),
        None if shown.starts_with('/') => shown.to_string(),
        None => return None,
    };
    Some(from_linux(distro, &linux))
}

/// Folders a Linux program of Null's is in, in a distribution: Null's own (servers, its
/// Node.js), then the usual ones, then the distribution's own PATH.
pub fn path_inside() -> String {
    "$HOME/.local/share/null/servers/node_modules/.bin:$HOME/.local/share/null/servers/bin:\
     $HOME/.local/share/null/node/bin:$HOME/.local/bin:$HOME/.cargo/bin:$HOME/go/bin:$PATH"
        .to_string()
}

/// `program args…` run inside `distro`, in `folder` (a Linux path), with Null's folders on its
/// PATH: through a login shell, so what the person's profile sets up is there too.
pub fn command(distro: &str, folder: Option<&str>, program: &str, args: &[&str]) -> std::process::Command {
    let mut command = crate::tools::command("wsl.exe");
    command.args(["-d", distro]);
    if let Some(folder) = folder {
        command.args(["--cd", folder]);
    }
    // ("$0" "$@": the program and its arguments as they are, nothing read twice.)
    let script = format!("PATH=\"{}\"; exec \"$0\" \"$@\"", path_inside());
    command.args(["--", "sh", "-lc", &script, program]).args(args);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_a_wsl_folder_and_its_linux_path() {
        let wsl = |p: &str| of(Path::new(p));
        let at = |d: &str, l: &str| Some(WslPath { distro: d.into(), linux: l.into() });
        assert_eq!(wsl(r"\\wsl.localhost\Ubuntu\home\me\proj"), at("Ubuntu", "/home/me/proj"));
        assert_eq!(wsl(r"\\wsl$\Ubuntu-24.04\home\me\"), at("Ubuntu-24.04", "/home/me"));
        assert_eq!(wsl(r"\\?\UNC\wsl.localhost\Debian\srv"), at("Debian", "/srv"));
        assert_eq!(wsl("//wsl.localhost/Ubuntu/home"), at("Ubuntu", "/home"));
        assert_eq!(wsl(r"\\WSL.LOCALHOST\Ubuntu"), at("Ubuntu", "/"));
        assert_eq!(wsl(r"\\server\share\x"), None);
        assert_eq!(wsl(r"C:\Users\me"), None);
        assert_eq!(wsl("/home/me"), None);
    }

    #[test]
    fn paths_go_both_ways() {
        let proj = windows_path("Ubuntu", "/home/me/proj");
        assert_eq!(proj, PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\proj"));
        assert_eq!(linux_path("Ubuntu", &proj), "/home/me/proj");
        assert_eq!(linux_path("Ubuntu", Path::new(r"C:\Users\me\a.txt")), "/mnt/c/Users/me/a.txt");
        assert_eq!(
            from_linux("Ubuntu", "/home/me/proj/src/a.rs"),
            PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\proj\src\a.rs")
        );
        assert_eq!(from_linux("Ubuntu", "/mnt/c/Users/me"), PathBuf::from(r"C:\Users\me"));
        assert_eq!(windows_path("Ubuntu", "/"), PathBuf::from(r"\\wsl.localhost\Ubuntu"));
    }

    #[test]
    fn reads_wsl_s_list_of_distributions() {
        let listed: Vec<u8> = "Ubuntu\r\nDebian\r\n\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(names_listed(&listed), ["Ubuntu", "Debian"]);
        assert!(names_listed(&[]).is_empty());
    }
}
