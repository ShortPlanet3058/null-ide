//! The language servers Null knows: which one serves each language, where to find it,
//! and how to install it when it's missing, into Null's own folder, never the system.
//! Servers that are npm packages bring Node.js along when the computer has none
//! (`node_runtime`); servers that are programs of their own are downloaded, checked against
//! the checksum their makers published (`download`).

use crate::download::{Asset, Packed, Target};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How to get a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    /// npm packages, installed into Null's folder (with Null's own Node.js if there's none).
    Npm(&'static [&'static str]),
    /// A rustup component when Rust is installed with rustup; else downloaded.
    RustupElse(&'static str, &'static Download),
    /// A Go module, built into Null's folder (needs Go).
    Go(&'static str),
    /// Comes with Xcode's command line tools on macOS; elsewhere, from the system's packages.
    Xcode,
    /// Xcode's on macOS; elsewhere downloaded.
    XcodeElse(&'static Download),
    /// A program of its own, downloaded.
    Download(&'static Download),
    /// A .NET tool, installed into Null's folder (needs the .NET SDK).
    Dotnet(&'static str),
    /// Comes with its language's own kit (the Dart SDK...): installed with it.
    Sdk { name: &'static str, from: &'static str },
}

/// A server that's a program of its own, for each system.
#[derive(Debug, PartialEq, Eq)]
pub struct Download {
    pub version: &'static str,
    pub assets: &'static [Asset],
    /// The program, once unpacked (no `.exe`).
    pub program: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Server {
    pub name: &'static str,
    /// The languages it serves, as people call them.
    pub label: &'static str,
    pub extensions: &'static [&'static str],
    /// Files it serves by their whole name (`Dockerfile`).
    pub file_names: &'static [&'static str],
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub install: Install,
}

const RUST_ANALYZER: Download = Download {
    version: "2026-10-12",
    program: "rust-analyzer",
    assets: &[
        Asset {
            target: Target::MacArm,
            url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-12/rust-analyzer-aarch64-apple-darwin.gz",
            sha256: "a2e45e828b37bfc4b507c2ca456e8b61096e0bdfbae225f3d33295546d1eeacf",
            packed: Packed::Gz,
        },
        Asset {
            target: Target::MacX64,
            url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-12/rust-analyzer-x86_64-apple-darwin.gz",
            sha256: "56a5698a2e0d495c3ec28155416db310c879f21a70f6f38db549166cf157cfab",
            packed: Packed::Gz,
        },
        Asset {
            target: Target::LinuxArm,
            url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-12/rust-analyzer-aarch64-unknown-linux-gnu.gz",
            sha256: "d7219d5d8f5537bb227b521157aebddfe7e4a90f4a85a74f31e53722f5691025",
            packed: Packed::Gz,
        },
        Asset {
            target: Target::LinuxX64,
            url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-12/rust-analyzer-x86_64-unknown-linux-gnu.gz",
            sha256: "040cab4e2abc490fadd95c3f6cbc60b941a78761dbc3acf0ec66fd7a97f31db3",
            packed: Packed::Gz,
        },
        Asset {
            target: Target::WindowsArm,
            url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-12/rust-analyzer-aarch64-pc-windows-msvc.zip",
            sha256: "b053585692c2848acfa8e9ddf5f85bdc3741f3b23c51b72717ba0b391910b5fe",
            packed: Packed::Zip,
        },
        Asset {
            target: Target::WindowsX64,
            url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-12/rust-analyzer-x86_64-pc-windows-msvc.zip",
            sha256: "77f7a907f0e6f179281bac38e2d08435c974db9a5b99de52c5c963aff52883ec",
            packed: Packed::Zip,
        },
    ],
};

/// (Windows on ARM runs the x64 build.)
const CLANGD: Download = Download {
    version: "23.1.0",
    program: "clangd_23.1.0/bin/clangd",
    assets: &[
        Asset {
            target: Target::LinuxX64,
            url: "https://github.com/clangd/clangd/releases/download/23.1.0/clangd-linux-23.1.0.zip",
            sha256: "e53b1a96196095faedb7642cf64964f7fb9ad4a0c1f00dd2c172a3d9dcbafdfd",
            packed: Packed::Zip,
        },
        Asset {
            target: Target::WindowsX64,
            url: "https://github.com/clangd/clangd/releases/download/23.1.0/clangd-windows-23.1.0.zip",
            sha256: "23412a240756a162e7b98a282f36aa2a23a88db5ce16a0cbc4fef7253768c810",
            packed: Packed::Zip,
        },
        Asset {
            target: Target::WindowsArm,
            url: "https://github.com/clangd/clangd/releases/download/23.1.0/clangd-windows-23.1.0.zip",
            sha256: "23412a240756a162e7b98a282f36aa2a23a88db5ce16a0cbc4fef7253768c810",
            packed: Packed::Zip,
        },
    ],
};

const MARKSMAN: Download = Download {
    version: "2026-02-08",
    program: "marksman",
    assets: &[
        Asset {
            target: Target::MacArm,
            url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman-macos",
            sha256: "6a801c17b5ac0dba69787c5282b3b3bd416e66c96253fae098d311c6bbd1833b",
            packed: Packed::Plain,
        },
        Asset {
            target: Target::MacX64,
            url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman-macos",
            sha256: "6a801c17b5ac0dba69787c5282b3b3bd416e66c96253fae098d311c6bbd1833b",
            packed: Packed::Plain,
        },
        Asset {
            target: Target::LinuxArm,
            url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman-linux-arm64",
            sha256: "db8e124527f7f8048e3e6c91821b9c52ef173d92c01e47d221bf1337afd962fb",
            packed: Packed::Plain,
        },
        Asset {
            target: Target::LinuxX64,
            url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman-linux-x64",
            sha256: "be5098e8213219269c47fc0d916a66fa31ce0602ec967475c722260aabf26087",
            packed: Packed::Plain,
        },
        Asset {
            target: Target::WindowsX64,
            url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman.exe",
            sha256: "a6d05beb08ebe41b0a9f09c98a438540421436fa5531424c22e0bb1d22529705",
            packed: Packed::Plain,
        },
        Asset {
            target: Target::WindowsArm,
            url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman.exe",
            sha256: "a6d05beb08ebe41b0a9f09c98a438540421436fa5531424c22e0bb1d22529705",
            packed: Packed::Plain,
        },
    ],
};

const LUA_LANGUAGE_SERVER: Download = Download {
    version: "3.19.2",
    program: "bin/lua-language-server",
    assets: &[
        Asset {
            target: Target::MacArm,
            url: "https://github.com/LuaLS/lua-language-server/releases/download/3.19.2/lua-language-server-3.19.2-darwin-arm64.tar.gz",
            sha256: "79149584320a23bd5fcd5bc31f47a1975acd8a4560a0e89036bb1016fa61687f",
            packed: Packed::TarGz,
        },
        Asset {
            target: Target::MacX64,
            url: "https://github.com/LuaLS/lua-language-server/releases/download/3.19.2/lua-language-server-3.19.2-darwin-x64.tar.gz",
            sha256: "6f2174ece5ec63bcd9d5b22b2c902445c1cd9045fafe48c62602448ebf4c09cc",
            packed: Packed::TarGz,
        },
        Asset {
            target: Target::LinuxArm,
            url: "https://github.com/LuaLS/lua-language-server/releases/download/3.19.2/lua-language-server-3.19.2-linux-arm64.tar.gz",
            sha256: "17b8e401a874440adeb649e5024c364f2bb9d15156c74a40e56d1ae9a1b2e4f2",
            packed: Packed::TarGz,
        },
        Asset {
            target: Target::LinuxX64,
            url: "https://github.com/LuaLS/lua-language-server/releases/download/3.19.2/lua-language-server-3.19.2-linux-x64.tar.gz",
            sha256: "93ad326b10f6c00911dd8c450fa098332dbd8f9f45d1052f5636a2d54479b947",
            packed: Packed::TarGz,
        },
        Asset {
            target: Target::WindowsX64,
            url: "https://github.com/LuaLS/lua-language-server/releases/download/3.19.2/lua-language-server-3.19.2-win32-x64.zip",
            sha256: "4a3dc851b3b31e7b75a23f645d1ba4a2137579a55f5fa4edbb17a3e2a6254044",
            packed: Packed::Zip,
        },
        Asset {
            target: Target::WindowsArm,
            url: "https://github.com/LuaLS/lua-language-server/releases/download/3.19.2/lua-language-server-3.19.2-win32-x64.zip",
            sha256: "4a3dc851b3b31e7b75a23f645d1ba4a2137579a55f5fa4edbb17a3e2a6254044",
            packed: Packed::Zip,
        },
    ],
};

pub const SERVERS: &[Server] = &[
    Server {
        name: "rust-analyzer",
        label: "Rust",
        extensions: &["rs"],
        file_names: &[],
        program: "rust-analyzer",
        args: &[],
        install: Install::RustupElse("rust-analyzer", &RUST_ANALYZER),
    },
    Server {
        name: "pyright",
        label: "Python",
        extensions: &["py", "pyi", "pyw"],
        file_names: &[],
        program: "pyright-langserver",
        args: &["--stdio"],
        install: Install::Npm(&["pyright"]),
    },
    Server {
        name: "typescript-language-server",
        label: "TypeScript and JavaScript",
        extensions: &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"],
        file_names: &[],
        program: "typescript-language-server",
        args: &["--stdio"],
        // (TypeScript 5: 7 is a native program without the tsserver this server runs.)
        install: Install::Npm(&["typescript-language-server", "typescript@5"]),
    },
    Server {
        name: "clangd",
        label: "C and C++",
        extensions: &["c", "h", "cc", "cpp", "cxx", "c++", "hpp", "hh", "hxx", "h++", "m", "mm"],
        file_names: &[],
        program: "clangd",
        args: &[],
        install: Install::XcodeElse(&CLANGD),
    },
    Server {
        name: "gopls",
        label: "Go",
        extensions: &["go"],
        file_names: &[],
        program: "gopls",
        args: &[],
        install: Install::Go("golang.org/x/tools/gopls"),
    },
    Server {
        name: "sourcekit-lsp",
        label: "Swift",
        extensions: &["swift"],
        file_names: &[],
        program: "sourcekit-lsp",
        args: &[],
        install: Install::Xcode,
    },
    Server {
        name: "bash-language-server",
        label: "Shell",
        extensions: &["sh", "bash", "zsh"],
        file_names: &[],
        program: "bash-language-server",
        args: &["start"],
        install: Install::Npm(&["bash-language-server"]),
    },
    Server {
        name: "vscode-html-language-server",
        label: "HTML",
        extensions: &["html", "htm", "xhtml"],
        file_names: &[],
        program: "vscode-html-language-server",
        args: &["--stdio"],
        install: Install::Npm(&["vscode-langservers-extracted"]),
    },
    Server {
        name: "vscode-css-language-server",
        label: "CSS",
        extensions: &["css", "scss", "less"],
        file_names: &[],
        program: "vscode-css-language-server",
        args: &["--stdio"],
        install: Install::Npm(&["vscode-langservers-extracted"]),
    },
    Server {
        name: "vscode-json-language-server",
        label: "JSON",
        extensions: &["json", "jsonc", "json5"],
        file_names: &[],
        program: "vscode-json-language-server",
        args: &["--stdio"],
        install: Install::Npm(&["vscode-langservers-extracted"]),
    },
    Server {
        name: "yaml-language-server",
        label: "YAML",
        extensions: &["yaml", "yml"],
        file_names: &[],
        program: "yaml-language-server",
        args: &["--stdio"],
        install: Install::Npm(&["yaml-language-server"]),
    },
    Server {
        name: "marksman",
        label: "Markdown",
        extensions: &["md", "markdown"],
        file_names: &[],
        program: "marksman",
        args: &["server"],
        install: Install::Download(&MARKSMAN),
    },
    Server {
        name: "lua-language-server",
        label: "Lua",
        extensions: &["lua"],
        file_names: &[],
        program: "lua-language-server",
        args: &[],
        install: Install::Download(&LUA_LANGUAGE_SERVER),
    },
    Server {
        name: "intelephense",
        label: "PHP",
        extensions: &["php", "phtml"],
        file_names: &[],
        program: "intelephense",
        args: &["--stdio"],
        install: Install::Npm(&["intelephense"]),
    },
    Server {
        name: "svelte-language-server",
        label: "Svelte",
        extensions: &["svelte"],
        file_names: &[],
        program: "svelteserver",
        args: &["--stdio"],
        install: Install::Npm(&["svelte-language-server", "typescript@5"]),
    },
    Server {
        name: "dockerfile-language-server",
        label: "Dockerfile",
        extensions: &["dockerfile"],
        file_names: &["Dockerfile", "Containerfile"],
        program: "docker-langserver",
        args: &["--stdio"],
        install: Install::Npm(&["dockerfile-language-server-nodejs"]),
    },
    Server {
        name: "csharp-ls",
        label: "C#",
        extensions: &["cs", "csx"],
        file_names: &[],
        program: "csharp-ls",
        args: &[],
        install: Install::Dotnet("csharp-ls"),
    },
    Server {
        name: "dart",
        label: "Dart",
        extensions: &["dart"],
        file_names: &[],
        program: "dart",
        args: &["language-server", "--protocol=lsp"],
        install: Install::Sdk { name: "the Dart SDK", from: "dart.dev" },
    },
];

pub fn for_path(path: &Path) -> Option<&'static Server> {
    let name = path.file_name()?.to_str()?;
    if let Some(server) = SERVERS.iter().find(|s| s.file_names.iter().any(|n| name.starts_with(n))) {
        return Some(server);
    }
    let ext = path.extension()?.to_str()?.to_lowercase();
    SERVERS.iter().find(|s| s.extensions.contains(&ext.as_str()))
}

/// The id servers use for a file's language.
pub fn language_id(path: &Path) -> &'static str {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.starts_with("Dockerfile") || name.starts_with("Containerfile") {
        return "dockerfile";
    }
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "rs" => "rust",
        "py" | "pyi" | "pyw" => "python",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" => "cpp",
        "m" => "objective-c",
        "mm" => "objective-cpp",
        "go" => "go",
        "swift" => "swift",
        "sh" | "bash" | "zsh" => "shellscript",
        "html" | "htm" | "xhtml" => "html",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "json" | "json5" => "json",
        "jsonc" => "jsonc",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "toml" => "toml",
        "lua" => "lua",
        "php" | "phtml" => "php",
        "svelte" => "svelte",
        "dockerfile" => "dockerfile",
        "cs" | "csx" => "csharp",
        "dart" => "dart",
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
    let tools = crate::tools::command("/usr/bin/xcode-select").arg("-p").output().is_ok_and(|o| o.status.success());
    if !tools {
        known.push((program, None));
        return None;
    }
    let found = crate::tools::command("/usr/bin/xcrun")
        .args(["--find", program])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .filter(|p| p.is_file());
    known.push((program, found.clone()));
    found
}

/// Where Null keeps a server it downloaded (whether or not it's there).
fn downloaded(server: &Server, download: &Download) -> Option<PathBuf> {
    let dir = crate::tools::data_dir()?.join("servers").join(server.name).join(download.version);
    Some(dir.join(crate::download::exe(download.program)))
}

/// Where the server is, if it's installed.
pub fn find(server: &Server) -> Option<PathBuf> {
    let on_mac = cfg!(target_os = "macos");
    match server.install {
        Install::Xcode | Install::XcodeElse(_) if on_mac => {
            // A real install on the PATH (Homebrew's LLVM...), else the one with Xcode. Never the
            // /usr/bin stub, which only asks to install the command line tools.
            return crate::tools::find(server.program)
                .filter(|p| !p.starts_with("/usr/bin"))
                .or_else(|| xcrun_find(server.program));
        }
        Install::XcodeElse(download) | Install::Download(download) | Install::RustupElse(_, download) => {
            return crate::tools::find(server.program)
                .filter(|p| !is_rustup_stub(server, p))
                .or_else(|| downloaded(server, download).filter(|p| p.is_file()));
        }
        Install::Dotnet(_) => {
            let own =
                crate::tools::data_dir().map(|d| d.join("servers/dotnet").join(crate::download::exe(server.program)));
            return crate::tools::find(server.program).or_else(|| own.filter(|p| p.is_file()));
        }
        _ => {}
    }
    crate::tools::find(server.program)
}

/// rustup's `rust-analyzer` in `~/.cargo/bin` is only a stand-in until its component is
/// added: no server yet.
fn is_rustup_stub(server: &Server, path: &Path) -> bool {
    if !matches!(server.install, Install::RustupElse(..)) || !path.components().any(|c| c.as_os_str() == ".cargo") {
        return false;
    }
    crate::tools::command(path).arg("--version").output().map(|o| !o.status.success()).unwrap_or(true)
}

/// Whether Null can install the server here, or what the person needs to do instead.
pub fn can_install(server: &Server) -> Result<(), String> {
    let no_build = || format!("There's no build of {} for this computer.", server.name);
    match server.install {
        Install::Npm(_) if crate::tools::find("npm").is_none() && !crate::node_runtime::available() => {
            Err(format!("{} needs Node.js to install. Get it from nodejs.org, then try again.", server.name))
        }
        Install::Go(_) if crate::tools::find("go").is_none() => {
            Err(format!("{} needs Go to install. Get it from go.dev, then try again.", server.name))
        }
        Install::RustupElse(_, download) | Install::Download(download)
            if crate::tools::find("rustup").is_none() && crate::download::for_here(download.assets).is_none() =>
        {
            Err(no_build())
        }
        Install::Xcode | Install::XcodeElse(_) if cfg!(target_os = "macos") => {
            Err(format!("{} comes with Xcode's command line tools: run “xcode-select --install”.", server.name))
        }
        Install::Xcode => Err(format!("Install {} with your system's package manager.", server.name)),
        Install::XcodeElse(download) if crate::download::for_here(download.assets).is_none() => {
            Err(format!("Install {} with your system's package manager.", server.name))
        }
        Install::Dotnet(_) if crate::tools::find("dotnet").is_none() => {
            Err(format!("{} needs the .NET SDK to install. Get it from dot.net, then try again.", server.name))
        }
        Install::Sdk { name, from } => Err(format!("{} comes with {name}: install it from {from}.", server.label)),
        _ => Ok(()),
    }
}

/// What installing brings along that the person might not expect (Node.js, when there's
/// none here).
pub fn install_note(server: &Server) -> Option<&'static str> {
    match server.install {
        Install::Npm(_) if crate::tools::find("npm").is_none() && crate::node_runtime::available() => {
            Some("Node.js comes with it, for Null's servers only")
        }
        _ => None,
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
    let run = |program: &Path, args: &[&str], env: &[(&str, &Path)]| -> Result<(), String> {
        let mut command = crate::tools::command(program);
        command.args(args).current_dir(&dir).env("PATH", crate::tools::search_path());
        for (key, value) in env {
            command.env(key, value);
        }
        let output = command.output().map_err(|e| format!("Couldn't run {}: {e}", program.display()))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).rev().take(3).collect();
        Err(format!("Installing {} failed: {}", server.name, detail.into_iter().rev().collect::<Vec<_>>().join(" ")))
    };
    let tool = |name: &str| crate::tools::find(name).ok_or_else(|| format!("“{name}” isn't installed."));
    let fetch = |download: &Download| -> Result<(), String> {
        let asset = crate::download::for_here(download.assets)
            .ok_or_else(|| format!("There's no build of {} for this computer.", server.name))?;
        let program = downloaded(server, download).ok_or("There's no folder to install into.")?;
        let folder = dir.join(server.name).join(download.version);
        let name = crate::download::exe(download.program.rsplit('/').next().unwrap_or(download.program));
        crate::download::install(asset, &folder, &name)?;
        // (Older versions it replaces go.)
        if let Ok(entries) = std::fs::read_dir(dir.join(server.name)) {
            for entry in entries.flatten().filter(|e| e.file_name() != download.version) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        program.is_file().then_some(()).ok_or_else(|| format!("{} was downloaded but can't be found.", server.name))
    };
    match server.install {
        Install::Npm(packages) => {
            let npm = crate::node_runtime::npm()?;
            let mut args = vec!["install", "--prefix", ".", "--no-audit", "--no-fund", "--loglevel=error"];
            args.extend(packages);
            run(&npm, &args, &[])
        }
        Install::RustupElse(component, download) => match crate::tools::find("rustup") {
            Some(rustup) => run(&rustup, &["component", "add", component], &[]),
            None => fetch(download),
        },
        Install::Go(module) => {
            let bin = dir.join("bin");
            run(&tool("go")?, &["install", &format!("{module}@latest")], &[("GOBIN", bin.as_path())])
        }
        Install::XcodeElse(download) if !cfg!(target_os = "macos") => fetch(download),
        Install::Download(download) => fetch(download),
        Install::Dotnet(package) => {
            let target = dir.join("dotnet");
            run(&tool("dotnet")?, &["tool", "install", package, "--tool-path", &target.to_string_lossy()], &[])
        }
        Install::Xcode | Install::XcodeElse(_) | Install::Sdk { .. } => {
            Err(can_install(server).err().unwrap_or_default())
        }
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
        assert_eq!(for_path(Path::new("x.lua")).map(|s| s.name), Some("lua-language-server"));
        assert_eq!(for_path(Path::new("index.php")).map(|s| s.name), Some("intelephense"));
        assert_eq!(for_path(Path::new("Dockerfile")).map(|s| s.name), Some("dockerfile-language-server"));
        assert_eq!(for_path(Path::new("Dockerfile.dev")).map(|s| s.name), Some("dockerfile-language-server"));
        assert_eq!(for_path(Path::new("App.cs")).map(|s| s.name), Some("csharp-ls"));
        assert_eq!(language_id(Path::new("x.h")), "c");
        assert_eq!(language_id(Path::new("x.tsx")), "typescriptreact");
        // Whatever the case of the extension.
        assert_eq!(language_id(Path::new("MAIN.CPP")), "cpp");
        assert_eq!(language_id(Path::new("Dockerfile")), "dockerfile");
        assert!(for_path(Path::new("notes.txt")).is_none());
    }

    /// Downloads them for real: `cargo test installs_for_real -- --ignored`.
    #[test]
    #[ignore]
    fn installs_for_real() {
        let data = crate::tools::data_dir().unwrap();
        for name in ["marksman", "lua-language-server"] {
            let server = SERVERS.iter().find(|s| s.name == name).unwrap();
            install(server).unwrap();
            let program = find(server).unwrap();
            assert!(program.starts_with(&data), "{name}: {}", program.display());
            let ran = crate::tools::command(&program).arg("--version").output().unwrap();
            assert!(ran.status.success(), "{name}: {}", String::from_utf8_lossy(&ran.stderr));
        }
    }

    #[test]
    fn downloads_are_pinned_and_checked() {
        for server in SERVERS {
            let (Install::Download(d) | Install::XcodeElse(d) | Install::RustupElse(_, d)) = server.install else {
                continue;
            };
            assert!(!d.assets.is_empty(), "{}", server.name);
            for asset in d.assets {
                assert!(asset.url.starts_with("https://github.com/"), "{}", asset.url);
                assert!(asset.url.contains(d.version), "{} is {}", asset.url, d.version);
                assert_eq!(asset.sha256.len(), 64, "{}", asset.url);
                assert!(asset.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            }
            // Every system has a build, except clangd on macOS (it's Xcode's there).
            let systems = if matches!(server.install, Install::XcodeElse(_)) { 3 } else { 6 };
            assert_eq!(d.assets.len(), systems, "{}", server.name);
        }
    }
}
