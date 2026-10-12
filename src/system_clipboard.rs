//! The clipboard Null copies to and pastes from: the Mac's, or, in a QA run (debug builds
//! with `NULL_QA`), one of the run's own, so a picture taken while the person works never
//! takes what they copied or puts something in its place.

use gpui::{App, ClipboardItem, Global};

/// The QA run's own clipboard.
#[derive(Default)]
struct Own(Option<ClipboardItem>);

impl Global for Own {}

/// Whether this is a QA run, kept apart from the person's clipboard (and keychain).
pub fn kept_apart() -> bool {
    cfg!(debug_assertions) && !cfg!(test) && std::env::var_os("NULL_QA").is_some()
}

pub fn write(cx: &mut App, item: ClipboardItem) {
    if kept_apart() {
        cx.set_global(Own(Some(item)));
    } else {
        cx.write_to_clipboard(item);
    }
}

pub fn read(cx: &App) -> Option<ClipboardItem> {
    if kept_apart() { cx.try_global::<Own>().and_then(|own| own.0.clone()) } else { cx.read_from_clipboard() }
}
