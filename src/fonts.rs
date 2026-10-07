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
    include_bytes!("../assets/fonts/instrument-sans/InstrumentSans-Italic.ttf"),
    include_bytes!("../assets/fonts/instrument-sans/InstrumentSans-MediumItalic.ttf"),
    include_bytes!("../assets/fonts/instrument-sans/InstrumentSans-SemiBoldItalic.ttf"),
    include_bytes!("../assets/fonts/instrument-sans/InstrumentSans-BoldItalic.ttf"),
];

/// The font families actually in use, after checking the ones in settings exist.
pub struct Fonts {
    pub code: SharedString,
    pub ui: SharedString,
}

impl Global for Fonts {}

/// How the code font joins characters. Never through `liga`: Geist Mono's joined glyphs
/// are one column wide, so whatever follows `->` or `!=` drifts left over it. Through
/// `calt` (how JetBrains Mono, Fira Code and Cascadia Code do it, column for column) when
/// ligatures are on.
pub fn code_features(ligatures: bool) -> gpui::FontFeatures {
    let mut off = vec![("liga".to_string(), 0)];
    if !ligatures {
        off.push(("calt".to_string(), 0));
    }
    gpui::FontFeatures(std::sync::Arc::new(off))
}

/// Code shown outside the editor (lists, cards, fields): the code font, joining
/// characters as the settings say.
pub trait CodeFont: gpui::Styled + Sized {
    fn code_font(self, cx: &App) -> Self {
        let ligatures = cx.global::<crate::settings::Settings>().ligatures;
        self.code_font_as(cx.global::<Fonts>().code.clone(), code_features(ligatures))
    }

    /// The code font `family`, joining characters by `features`.
    fn code_font_as(mut self, family: SharedString, features: gpui::FontFeatures) -> Self {
        let style = self.text_style().get_or_insert_with(Default::default);
        style.font_family = Some(family);
        style.font_features = Some(features);
        self
    }
}

impl<T: gpui::Styled> CodeFont for T {}

pub fn register(cx: &mut App) {
    if let Err(err) = cx.text_system().add_fonts(EMBEDDED.iter().map(|bytes| Cow::Borrowed(*bytes)).collect()) {
        eprintln!("null: couldn't load bundled fonts: {err}");
    }
}

/// The families of the fonts above: there, whatever the computer has.
const BUNDLED: &[&str] =
    &["Geist Mono", "CommitMono", "JetBrains Mono", "IBM Plex Mono", "Source Code Pro", "Instrument Sans"];

/// Picks the families to use for code and interface text.
pub fn apply(code: &str, ui: &str, cx: &mut App) {
    // Listing every installed font takes most of a tenth of a second at startup: only
    // for a family Null doesn't bring itself.
    let known = |name: &str| name == ".SystemUIFont" || BUNDLED.contains(&name);
    let installed = if known(code) && known(ui) { Vec::new() } else { cx.text_system().all_font_names() };
    let pick = |wanted: &str, fallback: &'static str| -> SharedString {
        if known(wanted) || installed.iter().any(|name| name == wanted) {
            wanted.to_string().into()
        } else {
            eprintln!("null: font \"{wanted}\" isn't installed, using {fallback}");
            fallback.into()
        }
    };
    let fonts = Fonts { code: pick(code, FALLBACK_CODE_FONT), ui: pick(ui, FALLBACK_UI_FONT) };
    cx.set_global(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The family a font file names (its `name` table, Windows Unicode records).
    fn family(font: &[u8]) -> Option<String> {
        let u16_at = |i: usize| Some(u16::from_be_bytes(font.get(i..i + 2)?.try_into().ok()?) as usize);
        let u32_at = |i: usize| Some(u32::from_be_bytes(font.get(i..i + 4)?.try_into().ok()?) as usize);
        let tables = u16_at(4)?;
        let name = (0..tables).map(|t| 12 + t * 16).find(|&at| font.get(at..at + 4) == Some(b"name"))?;
        let table = u32_at(name + 8)?;
        let (count, strings) = (u16_at(table + 2)?, table + u16_at(table + 4)?);
        // A typographic family (16) wins over the plain one (1) when both are there.
        let mut found = None;
        for r in (0..count).map(|r| table + 6 + r * 12) {
            let (platform, id) = (u16_at(r)?, u16_at(r + 6)?);
            if platform != 3 || !(id == 1 || id == 16) {
                continue;
            }
            let (len, offset) = (u16_at(r + 8)?, u16_at(r + 10)?);
            let units: Vec<u16> = font
                .get(strings + offset..strings + offset + len)?
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            let text = String::from_utf16(&units).ok()?;
            if id == 16 || found.is_none() {
                found = Some(text);
            }
        }
        found
    }

    #[test]
    fn bundled_families_are_the_fonts_names() {
        let families: Vec<String> = EMBEDDED.iter().filter_map(|f| family(f)).collect();
        assert_eq!(families.len(), EMBEDDED.len());
        for name in BUNDLED {
            assert!(families.iter().any(|f| f == name), "no bundled font is called {name}: {families:?}");
        }
    }
}
