use crate::fonts::{self, DEFAULT_CODE_FONT, DEFAULT_UI_FONT};
use crate::theme::{Theme, ThemeName};
use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_FONT_SIZE: f32 = 14.;
pub const MIN_FONT_SIZE: f32 = 9.;
pub const MAX_FONT_SIZE: f32 = 32.;

/// Everything Null remembers between launches. Stored as JSON so it can be
/// edited by hand; missing or unknown fields fall back to defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeName,
    /// Font family for code. Any installed font works; Geist Mono ships with Null.
    pub code_font: String,
    /// Font family for menus, tabs and the palette. Instrument Sans ships with Null.
    pub ui_font: String,
    pub font_size: f32,
    pub sidebar_visible: bool,
    /// Dim the title bar, sidebar and status bar while typing.
    pub fade_bars_while_typing: bool,
    /// Show suggestions while typing. Ctrl+Space asks for them either way.
    pub autocomplete: bool,
    /// Where AI answers come from. Off until a provider is chosen.
    pub ai: crate::ai::AiSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeName::Oled,
            code_font: DEFAULT_CODE_FONT.into(),
            ui_font: DEFAULT_UI_FONT.into(),
            font_size: DEFAULT_FONT_SIZE,
            sidebar_visible: true,
            fade_bars_while_typing: false,
            autocomplete: true,
            ai: Default::default(),
        }
    }
}

impl Global for Settings {}

impl Settings {
    /// `~/.config/null/settings.json` on macOS and Linux (or `$XDG_CONFIG_HOME/null`),
    /// `%APPDATA%\Null\settings.json` on Windows.
    pub fn path() -> Option<PathBuf> {
        let dir = if cfg!(target_os = "windows") {
            PathBuf::from(std::env::var_os("APPDATA")?).join("Null")
        } else if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            PathBuf::from(xdg).join("null")
        } else {
            PathBuf::from(std::env::var_os("HOME")?).join(".config").join("null")
        };
        Some(dir.join("settings.json"))
    }

    fn load() -> Self {
        let Some(path) = Self::path() else { return Self::default() };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|err| {
                eprintln!("null: ignoring {}: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    fn parse(text: &str) -> serde_json::Result<Self> {
        let mut settings: Self = serde_json::from_str(text)?;
        settings.font_size = settings.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        Ok(settings)
    }

    fn save(&self) {
        let Some(path) = Self::path() else { return };
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, serde_json::to_string_pretty(self).unwrap_or_default() + "\n"));
        if let Err(err) = result {
            eprintln!("null: couldn't save settings to {}: {err}", path.display());
        }
    }

    /// Makes sure the settings file exists, so it can be opened for editing.
    pub fn ensure_file(cx: &App) -> Option<PathBuf> {
        let path = Self::path()?;
        if !path.exists() {
            cx.global::<Self>().save();
        }
        Some(path)
    }
}

pub fn init(cx: &mut App) {
    fonts::register(cx);
    let settings = Settings::load();
    cx.set_global(Theme::named(settings.theme));
    fonts::apply(&settings.code_font, &settings.ui_font, cx);
    cx.set_global(settings);
}

/// Changes settings, saves them, and applies the theme.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Settings)) {
    let mut settings = cx.global::<Settings>().clone();
    change(&mut settings);
    settings.font_size = settings.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
    if settings != *cx.global::<Settings>() {
        settings.save();
        apply(settings, cx);
    }
}

/// Re-reads the settings file, after it was edited by hand.
pub fn reload(cx: &mut App) {
    let settings = Settings::load();
    if settings != *cx.global::<Settings>() {
        apply(settings, cx);
    }
}

fn apply(settings: Settings, cx: &mut App) {
    let old = cx.global::<Settings>();
    let theme_changed = settings.theme != old.theme;
    let fonts_changed = settings.code_font != old.code_font || settings.ui_font != old.ui_font;
    if theme_changed {
        cx.set_global(Theme::named(settings.theme));
    }
    if fonts_changed {
        fonts::apply(&settings.code_font, &settings.ui_font, cx);
    }
    cx.set_global(settings);
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let settings = Settings::parse(r#"{ "theme": "paper" }"#).unwrap();
        assert_eq!(settings.theme, ThemeName::Paper);
        assert_eq!(settings.font_size, DEFAULT_FONT_SIZE);
        assert!(settings.sidebar_visible);
    }

    #[test]
    fn font_size_is_clamped_and_unknown_fields_ignored() {
        let settings = Settings::parse(r#"{ "font_size": 400, "something_new": 1 }"#).unwrap();
        assert_eq!(settings.font_size, MAX_FONT_SIZE);
    }

    #[test]
    fn round_trips_through_json() {
        let settings = Settings { theme: ThemeName::Graphite, font_size: 16., ..Default::default() };
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(Settings::parse(&text).unwrap(), settings);
    }
}
