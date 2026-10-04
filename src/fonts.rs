use gpui::{App, Global, SharedString};
use std::borrow::Cow;

pub const DEFAULT_CODE_FONT: &str = "Geist Mono";
pub const DEFAULT_UI_FONT: &str = "Instrument Sans";

/// Code fonts offered, by family name and as shown. The first five ship with Null (on
/// every platform); the others appear when installed.
pub const CODE_FONTS: &[(&str, &str)] = &[
    ("Geist Mono", "Geist Mono"),
    ("CommitMono", "Commit Mono"),
    ("JetBrains Mono", "JetBrains Mono"),
    ("IBM Plex Mono", "IBM Plex Mono"),
    ("Source Code Pro", "Source Code Pro"),
    ("SF Mono", "SF Mono"),
    ("Menlo", "Menlo"),
    ("Fira Code", "Fira Code"),
    ("Cascadia Code", "Cascadia Code"),
    ("Consolas", "Consolas"),
];

/// Used when the font named in settings isn't installed.
#[cfg(target_os = "macos")]
const FALLBACK_CODE_FONT: &str = "Menlo";
#[cfg(target_os = "windows")]
const FALLBACK_CODE_FONT: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const FALLBACK_CODE_FONT: &str = "DejaVu Sans Mono";
const FALLBACK_UI_FONT: &str = ".SystemUIFont";

/// Bundled with Null under the SIL Open Font License (see `assets/fonts/*/OFL.txt`).
const EMBEDDED: &[&[u8]] = &[
    include_bytes!("../assets/fonts/geist-mono/GeistMono-Regular.ttf"),
    include_bytes!("../assets/fonts/geist-mono/GeistMono-Medium.ttf"),
    include_bytes!("../assets/fonts/geist-mono/GeistMono-SemiBold.ttf"),
    include_bytes!("../assets/fonts/commit-mono/CommitMono-400-Regular.ttf"),
    include_bytes!("../assets/fonts/commit-mono/CommitMono-700-Regular.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-SemiBold.ttf"),
    include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-SemiBold.ttf"),
    include_bytes!("../assets/fonts/source-code-pro/SourceCodePro-Regular.ttf"),
    include_bytes!("../assets/fonts/source-code-pro/SourceCodePro-Semibold.ttf"),
    include_bytes!("../assets/fonts/instrument-sans/InstrumentSans-Regular.ttf"),
    include_bytes!("../assets/fonts/instrument-sans/InstrumentSans-Medium.ttf"),
    include_bytes!("../assets/fonts/instrument-sans/InstrumentSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/instrument-sans/InstrumentSans-Bold.ttf"),
];

/// The font families actually in use, after checking the ones in settings exist.
pub struct Fonts {
    pub code: SharedString,
    pub ui: SharedString,
}

impl Global for Fonts {}

pub fn register(cx: &mut App) {
    if let Err(err) = cx.text_system().add_fonts(EMBEDDED.iter().map(|bytes| Cow::Borrowed(*bytes)).collect()) {
        eprintln!("null: couldn't load bundled fonts: {err}");
    }
}

/// Picks the families to use for code and interface text.
pub fn apply(code: &str, ui: &str, cx: &mut App) {
    let installed = cx.text_system().all_font_names();
    let pick = |wanted: &str, fallback: &'static str| -> SharedString {
        if wanted == ".SystemUIFont" || installed.iter().any(|name| name == wanted) {
            wanted.to_string().into()
        } else {
            eprintln!("null: font \"{wanted}\" isn't installed, using {fallback}");
            fallback.into()
        }
    };
    let fonts = Fonts { code: pick(code, FALLBACK_CODE_FONT), ui: pick(ui, FALLBACK_UI_FONT) };
    cx.set_global(fonts);
}
