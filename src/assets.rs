use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Icons compiled into the binary, so Null never depends on files next to it.
const ICONS: &[(&str, &[u8])] = &[
    ("icons/chevron-right.svg", include_bytes!("../assets/icons/chevron-right.svg")),
    ("icons/x.svg", include_bytes!("../assets/icons/x.svg")),
    ("icons/pin.svg", include_bytes!("../assets/icons/pin.svg")),
    ("icons/plus.svg", include_bytes!("../assets/icons/plus.svg")),
    ("icons/branch.svg", include_bytes!("../assets/icons/branch.svg")),
    ("icons/file-plus.svg", include_bytes!("../assets/icons/file-plus.svg")),
    ("icons/folder-plus.svg", include_bytes!("../assets/icons/folder-plus.svg")),
    ("icons/collapse.svg", include_bytes!("../assets/icons/collapse.svg")),
    ("icons/split.svg", include_bytes!("../assets/icons/split.svg")),
    ("icons/play.svg", include_bytes!("../assets/icons/play.svg")),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS.iter().find(|(name, _)| *name == path).map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS.iter().filter(|(name, _)| name.starts_with(path)).map(|(name, _)| (*name).into()).collect())
    }
}
