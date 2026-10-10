//! The language servers Null knows: which one serves each language, where to find it,
//! and how to install it when it's missing, into Null's own folder, never the system.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// How to get a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    /// npm packages, installed into Null's folder (needs Node.js).
    Npm(&'static [&'static str]),
    /// A rustup component.
    Rustup(&'static str),
    /// A Go module, built into Null's folder (needs Go).
    Go(&'static str),
    /// Comes with Xcode's command line tools on macOS; elsewhere, from the system's packages.
    Xcode,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Server {
    pub name: &'static str,
    /// The languages it serves, as people call them.
    pub label: &'static str,
    pub extensions: &'static [&'static str],
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub install: Install,
}

pub const SERVERS: &[Server] = &[
    Server {
        name: "rust-analyzer",
        label: "Rust",
        extensions: &["rs"],
        program: "rust-analyzer",
        args: &[],
        install: Install::Rustup("rust-analyzer"),
    },
    Server {
        name: "pyright",
        label: "Python",
        extensions: &["py", "pyi"],
        program: "pyright-langserver",
        args: &["--stdio"],
        install: Install::Npm(&["pyright"]),
    },
    Server {
        name: "typescript-language-server",
        label: "TypeScript and JavaScript",
        extensions: &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"],
        program: "typescript-language-server",
        args: &["--stdio"],
        install: Install::Npm(&["typescript-language-server", "typescript"]),
    },
    Server {
        name: "clangd",
        label: "C and C++",
        extensions: &["c", "h", "cc", "cpp", "cxx", "hpp", "hh", "hxx", "m", "mm"],
        program: "clangd",
        args: &[],
        install: Install::Xcode,
    },
    Server {
        name: "gopls",
        label: "Go",
        extensions: &["go"],
        program: "gopls",
        args: &[],
        install: Install::Go("golang.org/x/tools/gopls"),
    },
    Server {
        name: "sourcekit-lsp",
        label: "Swift",
        extensions: &["swift"],
        program: "sourcekit-lsp",
        args: &[],
        install: Install::Xcode,
    },
    Server {
        name: "bash-language-server",
        label: "Shell",
        extensions: &["sh", "bash", "zsh"],
        program: "bash-language-server",
        args: &["start"],
        install: Install::Npm(&["bash-language-server"]),
    },
    Server {
        name: "vscode-html-language-server",
        label: "HTML",
        extensions: &["html", "htm"],
        program: "vscode-html-language-server",
        args: &["--stdio"],
        install: Install::Npm(&["vscode-langservers-extracted"]),
    },
    Server {
        name: "vscode-css-language-server",
        label: "CSS",
        extensions: &["css", "scss", "less"],
        program: "vscode-css-language-server",
        args: &["--stdio"],
        install: Install::Npm(&["vscode-langservers-extracted"]),
    },
    Server {
        name: "vscode-json-language-server",
        label: "JSON",
        extensions: &["json", "jsonc"],
        program: "vscode-json-language-server",
        args: &["--stdio"],
        install: Install::Npm(&["vscode-langservers-extracted"]),
    },
    Server {
        name: "yaml-language-server",
        label: "YAML",
        extensions: &["yaml", "yml"],
        program: "yaml-language-server",
        args: &["--stdio"],
        install: Install::Npm(&["yaml-language-server"]),
    },
];

pub fn for_path(path: &Path) -> Option<&'static Server> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    SERVERS.iter().find(|s| s.extensions.contains(&ext.as_str()))
}

/// The id servers use for a file's language.
pub fn language_id(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "m" => "objective-c",
        "mm" => "objective-cpp",
        "go" => "go",
        "swift" => "swift",
        "sh" | "bash" | "zsh" => "shellscript",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "json" => "json",
        "jsonc" => "jsonc",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "toml" => "toml",
        _ => "plaintext",
    }
}

/// Programs that come with Xcode, found once through `xcrun`.
static XCODE: Mutex<Vec<(&'static str, Option<PathBuf>)>> = Mutex::new(Vec::new());

pub fn xcrun_find(program: &'static str) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut known = XCODE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, found)) = known.iter().find(|(p, _)| *p == program) {
        return found.clone();
    }
    // Without the command line tools, xcrun (and the stubs in /usr/bin) pop up an install
    // dialog: ask xcode-select first, which doesn't.
    let tools = Command::new("/usr/bin/xcode-select").arg("-p").output().is_ok_and(|o| o.status.success());
    if !tools {
        known.push((program, None));
        return None;
    }
    let found = Command::new("/usr/bin/xcrun")
        .args(["--find", program])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .filter(|p| p.is_file());
    known.push((program, found.clone()));
    found
}

/// Where the server is, if it's installed.
pub fn find(server: &Server) -> Option<PathBuf> {
    if server.install == Install::Xcode && cfg!(target_os = "macos") {
        // A real install on the PATH (Homebrew's LLVM...), else the one with Xcode. Never the
        // /usr/bin stub, which only asks to install the command line tools.
        return crate::tools::find(server.program)
            .filter(|p| !p.starts_with("/usr/bin"))
            .or_else(|| xcrun_find(server.program));
    }
    crate::tools::find(server.program)
}

/// Whether Null can install the server here, or what the person needs to do instead.
pub fn can_install(server: &Server) -> Result<(), String> {
    match server.install {
        Install::Npm(_) if crate::tools::find("npm").is_none() => {
            Err(format!("{} needs Node.js to install. Get it from nodejs.org, then try again.", server.name))
        }
        Install::Go(_) if crate::tools::find("go").is_none() => {
            Err(format!("{} needs Go to install. Get it from go.dev, then try again.", server.name))
        }
        Install::Rustup(_) if crate::tools::find("rustup").is_none() => {
            Err(format!("{} comes with Rust: install it from rustup.rs.", server.name))
        }
        Install::Xcode if cfg!(target_os = "macos") => {
            Err(format!("{} comes with Xcode's command line tools: run `xcode-select --install`.", server.name))
        }
        Install::Xcode => Err(format!("Install {} with your system's package manager.", server.name)),
        _ => Ok(()),
    }
}

/// Installs the server. Takes a while (it downloads): run it off the main thread.
pub fn install(server: &Server) -> Result<(), String> {
    // (A QA run uses what the person installed, and installs nothing: no downloads, no
    // change to their toolchains.)
    if crate::system_clipboard::kept_apart() {
        return Err("Nothing is installed in a QA run.".into());
    }
    can_install(server)?;
    let dir = crate::tools::data_dir().ok_or("There's no folder to install into.")?.join("servers");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    let run = |program: &str, args: &[&str], env: &[(&str, &Path)]| -> Result<(), String> {
        let path = crate::tools::find(program).ok_or_else(|| format!("`{program}` isn't installed."))?;
        let mut command = Command::new(path);
        command.args(args).current_dir(&dir).env("PATH", crate::tools::search_path());
        for (key, value) in env {
            command.env(key, value);
        }
        let output = command.output().map_err(|e| format!("Couldn't run {program}: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).rev().take(3).collect();
        Err(format!("Installing {} failed: {}", server.name, detail.into_iter().rev().collect::<Vec<_>>().join(" ")))
    };
    match server.install {
        Install::Npm(packages) => {
            let mut args = vec!["install", "--prefix", ".", "--no-audit", "--no-fund", "--loglevel=error"];
            args.extend(packages);
            run("npm", &args, &[])
        }
        Install::Rustup(component) => run("rustup", &["component", "add", component], &[]),
        Install::Go(module) => {
            let bin = dir.join("bin");
            run("go", &["install", &format!("{module}@latest")], &[("GOBIN", bin.as_path())])
        }
        Install::Xcode => Err(can_install(server).err().unwrap_or_default()),
    }?;
    find(server).map(|_| ()).ok_or_else(|| format!("{} was installed but can't be found.", server.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_finds_its_server() {
        assert_eq!(for_path(Path::new("a.py")).map(|s| s.name), Some("pyright"));
        assert_eq!(for_path(Path::new("main.C")).map(|s| s.name), Some("clangd"));
        assert_eq!(for_path(Path::new("x.h")).map(|s| s.name), Some("clangd"));
        assert_eq!(language_id(Path::new("x.h")), "c");
        assert_eq!(language_id(Path::new("x.tsx")), "typescriptreact");
        assert!(for_path(Path::new("notes.txt")).is_none());
    }
}
