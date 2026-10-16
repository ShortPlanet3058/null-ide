//! A file's language as a small logo, where a dot marks a file (the files, ⌘P): one colour,
//! the dot's, so the list stays as quiet as it was. Files of other kinds keep the dot.
//! The logos are Simple Icons' (CC0), in assets/icons/lang.

use gpui::{AnyElement, Hsla, IntoElement, ParentElement, Styled, div, px, svg};
use std::path::Path;

/// The logos there are, by name (assets/icons/lang/<name>.svg).
pub const NAMES: &[&str] = &[
    "astro",
    "c",
    "clojure",
    "cmake",
    "cplusplus",
    "css",
    "dart",
    "docker",
    "elixir",
    "erlang",
    "fsharp",
    "git",
    "gnubash",
    "go",
    "graphql",
    "haskell",
    "html5",
    "javascript",
    "json",
    "julia",
    "kotlin",
    "less",
    "lua",
    "markdown",
    "nim",
    "ocaml",
    "openjdk",
    "perl",
    "php",
    "prisma",
    "python",
    "r",
    "react",
    "ruby",
    "rust",
    "sass",
    "scala",
    "solidity",
    "svelte",
    "swift",
    "terraform",
    "toml",
    "typescript",
    "vuedotjs",
    "yaml",
    "zig",
];

/// Extensions (lower case) and their logo.
const BY_EXTENSION: &[(&str, &str)] = &[
    ("rs", "rust"),
    ("py", "python"),
    ("pyi", "python"),
    ("pyw", "python"),
    ("ipynb", "python"),
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("jsx", "react"),
    ("tsx", "react"),
    ("ts", "typescript"),
    ("mts", "typescript"),
    ("cts", "typescript"),
    ("json", "json"),
    ("jsonc", "json"),
    ("toml", "toml"),
    ("md", "markdown"),
    ("markdown", "markdown"),
    ("html", "html5"),
    ("htm", "html5"),
    ("css", "css"),
    ("scss", "sass"),
    ("sass", "sass"),
    ("less", "less"),
    ("rb", "ruby"),
    ("php", "php"),
    ("cpp", "cplusplus"),
    ("cc", "cplusplus"),
    ("cxx", "cplusplus"),
    ("hpp", "cplusplus"),
    ("hh", "cplusplus"),
    ("hxx", "cplusplus"),
    ("c", "c"),
    ("h", "c"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("sh", "gnubash"),
    ("bash", "gnubash"),
    ("zsh", "gnubash"),
    ("java", "openjdk"),
    ("lua", "lua"),
    ("go", "go"),
    ("swift", "swift"),
    ("kt", "kotlin"),
    ("kts", "kotlin"),
    ("dart", "dart"),
    ("dockerfile", "docker"),
    ("svelte", "svelte"),
    ("vue", "vuedotjs"),
    ("graphql", "graphql"),
    ("gql", "graphql"),
    ("zig", "zig"),
    ("ex", "elixir"),
    ("exs", "elixir"),
    ("hs", "haskell"),
    ("scala", "scala"),
    ("r", "r"),
    ("jl", "julia"),
    ("tf", "terraform"),
    ("prisma", "prisma"),
    ("astro", "astro"),
    ("erl", "erlang"),
    ("fs", "fsharp"),
    ("fsx", "fsharp"),
    ("ml", "ocaml"),
    ("mli", "ocaml"),
    ("clj", "clojure"),
    ("cljs", "clojure"),
    ("pl", "perl"),
    ("pm", "perl"),
    ("nim", "nim"),
    ("cmake", "cmake"),
    ("sol", "solidity"),
];

/// Whole names (lower case) that say more than their extension.
const BY_NAME: &[(&str, &str)] = &[
    ("dockerfile", "docker"),
    ("containerfile", "docker"),
    ("cmakelists.txt", "cmake"),
    (".gitignore", "git"),
    (".gitattributes", "git"),
    (".gitmodules", "git"),
    (".gitkeep", "git"),
];

/// The asset of `path`'s logo, if its kind has one.
pub fn for_path(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?.to_lowercase();
    let logo = BY_NAME.iter().find(|(n, _)| *n == name).map(|(_, l)| *l).or_else(|| {
        // (`Dockerfile.dev`, `api.Dockerfile`.)
        if name.starts_with("dockerfile.") {
            return Some("docker");
        }
        let extension = name.rsplit_once('.')?.1;
        BY_EXTENSION.iter().find(|(e, _)| *e == extension).map(|(_, l)| *l)
    })?;
    ASSETS.iter().find(|a| a.ends_with(&format!("/{logo}.svg"))).copied()
}

/// The assets' paths, as `assets` serves them.
static ASSETS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| {
    NAMES.iter().map(|n| &*Box::leak(format!("icons/lang/{n}.svg").into_boxed_str())).collect()
});

/// What marks `path` in a list: its logo, or a dot, in `color`.
pub fn marker(path: &Path, color: Hsla) -> AnyElement {
    match for_path(path) {
        Some(logo) => svg().path(logo).flex_none().size(px(11.)).text_color(color).into_any_element(),
        None => div().flex_none().size(px(5.)).rounded(px(2.)).bg(color).into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_s_logo_by_its_extension_or_name() {
        let logo = |p: &str| for_path(Path::new(p));
        assert_eq!(logo("src/main.rs"), Some("icons/lang/rust.svg"));
        assert_eq!(logo("App.TSX"), Some("icons/lang/react.svg"));
        assert_eq!(logo("Dockerfile"), Some("icons/lang/docker.svg"));
        assert_eq!(logo("Dockerfile.dev"), Some("icons/lang/docker.svg"));
        assert_eq!(logo(".gitignore"), Some("icons/lang/git.svg"));
        assert_eq!(logo("CMakeLists.txt"), Some("icons/lang/cmake.svg"));
        assert_eq!(logo("notes.txt"), None);
        assert_eq!(logo("Makefile"), None);
    }

    /// Every logo named is one the app has, and every one it has is named.
    #[test]
    fn every_logo_is_there() {
        for (_, logo) in BY_EXTENSION.iter().chain(BY_NAME) {
            assert!(NAMES.contains(logo), "{logo} isn't among the logos");
        }
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons/lang");
        for name in NAMES {
            assert!(dir.join(format!("{name}.svg")).is_file(), "no {name}.svg");
        }
        let svgs = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "svg"));
        assert_eq!(svgs.count(), NAMES.len());
    }
}
