use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Icons compiled into the binary, so Null never depends on files next to it.
const ICONS: &[(&str, &[u8])] = &[
    ("icons/chevron-right.svg", include_bytes!("../assets/icons/chevron-right.svg")),
    ("icons/x.svg", include_bytes!("../assets/icons/x.svg")),
    ("icons/pin.svg", include_bytes!("../assets/icons/pin.svg")),
    ("icons/lock.svg", include_bytes!("../assets/icons/lock.svg")),
    ("icons/plus.svg", include_bytes!("../assets/icons/plus.svg")),
    ("icons/branch.svg", include_bytes!("../assets/icons/branch.svg")),
    ("icons/file-plus.svg", include_bytes!("../assets/icons/file-plus.svg")),
    ("icons/folder-plus.svg", include_bytes!("../assets/icons/folder-plus.svg")),
    ("icons/collapse.svg", include_bytes!("../assets/icons/collapse.svg")),
    ("icons/split.svg", include_bytes!("../assets/icons/split.svg")),
    ("icons/play.svg", include_bytes!("../assets/icons/play.svg")),
    ("icons/menu.svg", include_bytes!("../assets/icons/menu.svg")),
    ("icons/window-minimize.svg", include_bytes!("../assets/icons/window-minimize.svg")),
    ("icons/window-maximize.svg", include_bytes!("../assets/icons/window-maximize.svg")),
    ("icons/window-restore.svg", include_bytes!("../assets/icons/window-restore.svg")),
];

/// The languages' logos (see `file_icons`).
macro_rules! lang {
    ($name:literal) => {
        (concat!("icons/lang/", $name, ".svg"), include_bytes!(concat!("../assets/icons/lang/", $name, ".svg")) as &[u8])
    };
}

const LANGUAGES: &[(&str, &[u8])] = &[
    lang!("astro"),
    lang!("c"),
    lang!("clojure"),
    lang!("cmake"),
    lang!("cplusplus"),
    lang!("css"),
    lang!("dart"),
    lang!("docker"),
    lang!("elixir"),
    lang!("erlang"),
    lang!("fsharp"),
    lang!("git"),
    lang!("gnubash"),
    lang!("go"),
    lang!("graphql"),
    lang!("haskell"),
    lang!("html5"),
    lang!("javascript"),
    lang!("json"),
    lang!("julia"),
    lang!("kotlin"),
    lang!("less"),
    lang!("lua"),
    lang!("markdown"),
    lang!("nim"),
    lang!("ocaml"),
    lang!("openjdk"),
    lang!("perl"),
    lang!("php"),
    lang!("prisma"),
    lang!("python"),
    lang!("r"),
    lang!("react"),
    lang!("ruby"),
    lang!("rust"),
    lang!("sass"),
    lang!("scala"),
    lang!("solidity"),
    lang!("svelte"),
    lang!("swift"),
    lang!("terraform"),
    lang!("toml"),
    lang!("typescript"),
    lang!("vuedotjs"),
    lang!("yaml"),
    lang!("zig"),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS.iter().chain(LANGUAGES).find(|(name, _)| *name == path).map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .chain(LANGUAGES)
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| (*name).into())
            .collect())
    }
}
