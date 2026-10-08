use gpui::{Global, Hsla, rgb, rgba};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    /// True black with amber: Null's own (once called "OLED").
    #[serde(alias = "oled")]
    Null,
    /// Soft charcoal (once called "Graphite").
    #[serde(alias = "graphite")]
    Ash,
    Midnight,
    Moss,
    Paper,
    Dune,
}

impl ThemeName {
    pub const ALL: [ThemeName; 6] =
        [ThemeName::Null, ThemeName::Ash, ThemeName::Midnight, ThemeName::Moss, ThemeName::Paper, ThemeName::Dune];

    pub fn label(self) -> &'static str {
        match self {
            ThemeName::Null => "Null",
            ThemeName::Ash => "Ash",
            ThemeName::Midnight => "Midnight",
            ThemeName::Moss => "Moss",
            ThemeName::Paper => "Paper",
            ThemeName::Dune => "Dune",
        }
    }

    /// Whether it's one of the light ones (dark text on a pale ground).
    pub fn is_light(self) -> bool {
        matches!(self, ThemeName::Paper | ThemeName::Dune)
    }

    /// A few words on it, for the choosers.
    pub fn note(self) -> &'static str {
        match self {
            ThemeName::Null => "True black, amber caret",
            ThemeName::Ash => "Soft charcoal for long sessions",
            ThemeName::Midnight => "Deep blue, cool accents",
            ThemeName::Moss => "Dark green-grey, calm and warm",
            ThemeName::Paper => "Light, for bright rooms",
            ThemeName::Dune => "Warm sand, terracotta caret",
        }
    }
}

/// What a span of code is, as far as coloring goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    Plain,
    Keyword,
    Type,
    Function,
    Macro,
    String,
    Number,
    Comment,
    Punctuation,
    Attribute,
    Property,
    /// A note left in a comment: TODO, FIXME, HACK, XXX.
    Note,
    /// A bracket, by how deep it is (0, 1, 2, then round again): coloured bracket pairs.
    Bracket(u8),
}

#[derive(Clone)]
pub struct Theme {
    pub background: Hsla,
    pub surface: Hsla,
    pub hairline: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub caret: Hsla,
    pub accent_soft: Hsla,
    /// Floating layers like the command palette.
    pub raised: Hsla,
    /// A border that should read (key caps, buttons), where `hairline` only separates.
    pub line_strong: Hsla,
    /// Text on the amber accent.
    pub on_accent: Hsla,
    /// Set into a raised panel: code blocks in cards and notes.
    pub sunken: Hsla,
    /// Dims what is behind a floating layer.
    pub scrim: Hsla,
    pub selection: Hsla,
    /// Other occurrences of the search, behind the current one.
    pub find_match: Hsla,
    pub error: Hsla,
    pub warning: Hsla,
    pub git_added: Hsla,
    pub git_modified: Hsla,
    pub git_deleted: Hsla,
    /// The 16 terminal colors: black, red, green, yellow, blue, magenta, cyan, white, then bright versions.
    pub ansi: [Hsla; 16],
    pub current_line: Hsla,
    keyword: Hsla,
    ty: Hsla,
    function: Hsla,
    macro_: Hsla,
    string: Hsla,
    number: Hsla,
    comment: Hsla,
    punctuation: Hsla,
    attribute: Hsla,
    property: Hsla,
}

impl Global for Theme {}

impl Theme {
    pub fn named(name: ThemeName) -> Self {
        match name {
            ThemeName::Null => Self::oled(),
            ThemeName::Ash => Self::graphite(),
            ThemeName::Midnight => Self::midnight(),
            ThemeName::Moss => Self::moss(),
            ThemeName::Paper => Self::paper(),
            ThemeName::Dune => Self::dune(),
        }
    }

    /// The default theme: true black, so OLED pixels stay switched off.
    pub fn oled() -> Self {
        Self {
            background: rgb(0x000000).into(),
            surface: rgb(0x08080a).into(),
            hairline: rgb(0x1d1d22).into(),
            foreground: rgb(0xebe8e2).into(),
            muted: rgb(0x8b8a92).into(),
            faint: rgb(0x46464e).into(),
            caret: rgb(0xf2b35b).into(),
            accent_soft: rgba(0xf2b35b1f).into(),
            raised: rgb(0x141418).into(),
            line_strong: rgb(0x2c2c33).into(),
            on_accent: rgb(0x1b1206).into(),
            sunken: rgb(0x0b0b0d).into(),
            scrim: rgba(0x00000073).into(),
            selection: rgba(0xf2b35b38).into(),
            find_match: rgba(0xffffff1f).into(),
            error: rgb(0xe88d8d).into(),
            warning: rgb(0xe6d27e).into(),
            git_added: rgb(0x6cc58c).into(),
            git_modified: rgb(0x7fa8e0).into(),
            git_deleted: rgb(0xe88d8d).into(),
            ansi: [
                rgb(0x1c1c21).into(),
                rgb(0xe88d8d).into(),
                rgb(0x86d6a2).into(),
                rgb(0xe6d27e).into(),
                rgb(0x7fa8e0).into(),
                rgb(0xc6a6f6).into(),
                rgb(0x7fd2c7).into(),
                rgb(0xd9d6cf).into(),
                rgb(0x5f626e).into(),
                rgb(0xf0a0a0).into(),
                rgb(0xa2e3b6).into(),
                rgb(0xf0de96).into(),
                rgb(0x9cbdf0).into(),
                rgb(0xd6bcfa).into(),
                rgb(0x9ae0d7).into(),
                rgb(0xf4f2ed).into(),
            ],
            current_line: rgb(0x0b0b0e).into(),
            keyword: rgb(0xc6a6f6).into(),
            ty: rgb(0x87c7e1).into(),
            function: rgb(0xf1d9a9).into(),
            macro_: rgb(0x7fd2c7).into(),
            string: rgb(0xa9d59c).into(),
            number: rgb(0xf3a37e).into(),
            comment: rgb(0x5f626e).into(),
            punctuation: rgb(0x8d8c96).into(),
            attribute: rgb(0x9aa6ba).into(),
            property: rgb(0xd9d6cf).into(),
        }
    }

    /// Soft charcoal for long sessions.
    pub fn graphite() -> Self {
        Self {
            background: rgb(0x131417).into(),
            surface: rgb(0x18191d).into(),
            hairline: rgb(0x26272d).into(),
            foreground: rgb(0xe4e3e0).into(),
            muted: rgb(0x8f8f98).into(),
            faint: rgb(0x4d4e56).into(),
            caret: rgb(0xefb565).into(),
            accent_soft: rgba(0xefb5651f).into(),
            raised: rgb(0x212227).into(),
            line_strong: rgb(0x35363d).into(),
            on_accent: rgb(0x1b1206).into(),
            sunken: rgb(0x18191d).into(),
            scrim: rgba(0x00000052).into(),
            selection: rgba(0xefb56533).into(),
            find_match: rgba(0xffffff1f).into(),
            error: rgb(0xea9393).into(),
            warning: rgb(0xe3d184).into(),
            git_added: rgb(0x72c690).into(),
            git_modified: rgb(0x84abe0).into(),
            git_deleted: rgb(0xea9393).into(),
            ansi: [
                rgb(0x2a2b31).into(),
                rgb(0xea9393).into(),
                rgb(0x8ad7a5).into(),
                rgb(0xe3d184).into(),
                rgb(0x84abe0).into(),
                rgb(0xc4a8f0).into(),
                rgb(0x84cfc5).into(),
                rgb(0xd6d4cf).into(),
                rgb(0x676a75).into(),
                rgb(0xf2a6a6).into(),
                rgb(0xa6e2bb).into(),
                rgb(0xeedc9a).into(),
                rgb(0xa0c0ef).into(),
                rgb(0xd3bff6).into(),
                rgb(0xa0dcd4).into(),
                rgb(0xf2f1ee).into(),
            ],
            current_line: rgb(0x18191d).into(),
            keyword: rgb(0xc4a8f0).into(),
            ty: rgb(0x8ac6dd).into(),
            function: rgb(0xeed9b0).into(),
            macro_: rgb(0x84cfc5).into(),
            string: rgb(0xacd3a0).into(),
            number: rgb(0xf0a685).into(),
            comment: rgb(0x676a75).into(),
            punctuation: rgb(0x91909a).into(),
            attribute: rgb(0x9ca7b8).into(),
            property: rgb(0xd6d4cf).into(),
        }
    }

    /// Light, for bright rooms.
    pub fn paper() -> Self {
        Self {
            background: rgb(0xfcfcfb).into(),
            surface: rgb(0xf4f4f2).into(),
            hairline: rgb(0xe6e5e1).into(),
            foreground: rgb(0x1d1e22).into(),
            muted: rgb(0x6a6b72).into(),
            faint: rgb(0xb0b1b7).into(),
            caret: rgb(0xb4620c).into(),
            accent_soft: rgba(0xb4620c1a).into(),
            raised: rgb(0xffffff).into(),
            line_strong: rgb(0xd4d3ce).into(),
            on_accent: rgb(0xffffff).into(),
            sunken: rgb(0xf4f4f2).into(),
            scrim: rgba(0x14141929).into(),
            selection: rgba(0xb4620c2b).into(),
            find_match: rgba(0x1d1e221a).into(),
            error: rgb(0xb23a3a).into(),
            warning: rgb(0x9a7400).into(),
            git_added: rgb(0x2e8b57).into(),
            git_modified: rgb(0x2f6db5).into(),
            git_deleted: rgb(0xb23a3a).into(),
            ansi: [
                rgb(0x1d1e22).into(),
                rgb(0xb23a3a).into(),
                rgb(0x2e7d4b).into(),
                rgb(0x8a6a00).into(),
                rgb(0x2f6db5).into(),
                rgb(0x7a3ec8).into(),
                rgb(0x0f7f78).into(),
                rgb(0x8a8b92).into(),
                rgb(0x6a6b72).into(),
                rgb(0xc94545).into(),
                rgb(0x3a9a5c).into(),
                rgb(0xa07800).into(),
                rgb(0x3d7fcf).into(),
                rgb(0x8e4fd8).into(),
                rgb(0x149a92).into(),
                rgb(0xb0b1b7).into(),
            ],
            current_line: rgb(0xf4f3ef).into(),
            keyword: rgb(0x7a3ec8).into(),
            ty: rgb(0x1e6c92).into(),
            function: rgb(0x8c4a00).into(),
            macro_: rgb(0x0f7f78).into(),
            string: rgb(0x3a7a2e).into(),
            number: rgb(0xb4461e).into(),
            comment: rgb(0x9a9ca3).into(),
            punctuation: rgb(0x6f7077).into(),
            attribute: rgb(0x5c6b82).into(),
            property: rgb(0x3b3d44).into(),
        }
    }

    /// Deep blue-black with a soft cyan caret.
    pub fn midnight() -> Self {
        Self {
            background: rgb(0x0b0e14).into(),
            surface: rgb(0x0f131b).into(),
            hairline: rgb(0x1c2230).into(),
            foreground: rgb(0xdfe4ee).into(),
            muted: rgb(0x8590a6).into(),
            faint: rgb(0x414b60).into(),
            caret: rgb(0x7cc4e8).into(),
            accent_soft: rgba(0x7cc4e81f).into(),
            raised: rgb(0x151a24).into(),
            line_strong: rgb(0x29303f).into(),
            on_accent: rgb(0x07141c).into(),
            sunken: rgb(0x0e121a).into(),
            scrim: rgba(0x0003086b).into(),
            selection: rgba(0x7cc4e833).into(),
            find_match: rgba(0xffffff1c).into(),
            current_line: rgb(0x0f131b).into(),
            keyword: rgb(0xb7a3f2).into(),
            ty: rgb(0x8bd0e6).into(),
            function: rgb(0xe9d7a8).into(),
            macro_: rgb(0x7fd8cf).into(),
            string: rgb(0xa4d4a3).into(),
            number: rgb(0xf0a682).into(),
            comment: rgb(0x58627a).into(),
            punctuation: rgb(0x8890a3).into(),
            attribute: rgb(0x9aa8c2).into(),
            property: rgb(0xd2d9e6).into(),
            ..Self::graphite()
        }
    }

    /// Dark green-grey with a sage caret.
    pub fn moss() -> Self {
        Self {
            background: rgb(0x101311).into(),
            surface: rgb(0x141815).into(),
            hairline: rgb(0x222823).into(),
            foreground: rgb(0xe2e5dd).into(),
            muted: rgb(0x8c9488).into(),
            faint: rgb(0x474f47).into(),
            caret: rgb(0xb5cf85).into(),
            accent_soft: rgba(0xb5cf851f).into(),
            raised: rgb(0x1a1f1b).into(),
            line_strong: rgb(0x2f3630).into(),
            on_accent: rgb(0x111a06).into(),
            sunken: rgb(0x131714).into(),
            scrim: rgba(0x0004016b).into(),
            selection: rgba(0xb5cf8533).into(),
            find_match: rgba(0xffffff1c).into(),
            current_line: rgb(0x141815).into(),
            keyword: rgb(0xd0a6d8).into(),
            ty: rgb(0x8ec9c0).into(),
            function: rgb(0xe8d39c).into(),
            macro_: rgb(0x9fd3b0).into(),
            string: rgb(0xb7d48f).into(),
            number: rgb(0xeba37c).into(),
            comment: rgb(0x606b5f).into(),
            punctuation: rgb(0x8f968b).into(),
            attribute: rgb(0xa2ad9c).into(),
            property: rgb(0xd8dcd0).into(),
            ..Self::graphite()
        }
    }

    /// Warm sand with a terracotta caret.
    pub fn dune() -> Self {
        Self {
            background: rgb(0xf6f1e7).into(),
            surface: rgb(0xefe8da).into(),
            hairline: rgb(0xe2d9c7).into(),
            foreground: rgb(0x2b2620).into(),
            muted: rgb(0x75695a).into(),
            faint: rgb(0xb6aa97).into(),
            caret: rgb(0xb5532d).into(),
            accent_soft: rgba(0xb5532d1a).into(),
            raised: rgb(0xfbf8f1).into(),
            line_strong: rgb(0xd5c9b3).into(),
            on_accent: rgb(0xffffff).into(),
            sunken: rgb(0xefe8da).into(),
            scrim: rgba(0x2b201429).into(),
            selection: rgba(0xb5532d2b).into(),
            find_match: rgba(0x2b26201a).into(),
            current_line: rgb(0xf0e9dc).into(),
            keyword: rgb(0x8a3f8f).into(),
            ty: rgb(0x2c6e7f).into(),
            function: rgb(0x8f4a10).into(),
            macro_: rgb(0x217a6b).into(),
            string: rgb(0x527a26).into(),
            number: rgb(0xb5532d).into(),
            comment: rgb(0xa2967f).into(),
            punctuation: rgb(0x7a6e5e).into(),
            attribute: rgb(0x6a6455).into(),
            property: rgb(0x4a4136).into(),
            ..Self::paper()
        }
    }

    /// The colour a theme file names (`"keyword"`, `"background"`), to read or set.
    fn slot(&mut self, name: &str) -> Option<&mut Hsla> {
        Some(match name {
            "background" => &mut self.background,
            "surface" => &mut self.surface,
            "hairline" => &mut self.hairline,
            "foreground" => &mut self.foreground,
            "muted" => &mut self.muted,
            "faint" => &mut self.faint,
            "caret" => &mut self.caret,
            "accent_soft" => &mut self.accent_soft,
            "raised" => &mut self.raised,
            "line_strong" => &mut self.line_strong,
            "on_accent" => &mut self.on_accent,
            "sunken" => &mut self.sunken,
            "scrim" => &mut self.scrim,
            "selection" => &mut self.selection,
            "find_match" => &mut self.find_match,
            "error" => &mut self.error,
            "warning" => &mut self.warning,
            "git_added" => &mut self.git_added,
            "git_modified" => &mut self.git_modified,
            "git_deleted" => &mut self.git_deleted,
            "current_line" => &mut self.current_line,
            "keyword" => &mut self.keyword,
            "type" => &mut self.ty,
            "function" => &mut self.function,
            "macro" => &mut self.macro_,
            "string" => &mut self.string,
            "number" => &mut self.number,
            "comment" => &mut self.comment,
            "punctuation" => &mut self.punctuation,
            "attribute" => &mut self.attribute,
            "property" => &mut self.property,
            _ => return None,
        })
    }

    pub fn syntax(&self, syntax: Syntax) -> Hsla {
        match syntax {
            Syntax::Plain => self.foreground,
            Syntax::Keyword => self.keyword,
            Syntax::Type => self.ty,
            Syntax::Function => self.function,
            Syntax::Macro => self.macro_,
            Syntax::String => self.string,
            Syntax::Number => self.number,
            Syntax::Comment => self.comment,
            Syntax::Punctuation => self.punctuation,
            Syntax::Attribute => self.attribute,
            Syntax::Property => self.property,
            Syntax::Note => self.caret,
            // Three of the theme's own colours, so pairs stand apart and still belong.
            Syntax::Bracket(depth) => [self.keyword, self.function, self.ty][depth as usize % 3],
        }
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn old_theme_names_still_load() {
        let name = |json: &str| serde_json::from_str::<ThemeName>(json).unwrap();
        assert_eq!(name("\"oled\""), ThemeName::Null);
        assert_eq!(name("\"graphite\""), ThemeName::Ash);
        assert_eq!(name("\"null\""), ThemeName::Null);
        assert_eq!(serde_json::to_string(&ThemeName::Null).unwrap(), "\"null\"");
        // Every theme builds, with its own background.
        let backgrounds: std::collections::HashSet<String> =
            ThemeName::ALL.iter().map(|n| format!("{:?}", Theme::named(*n).background)).collect();
        assert_eq!(backgrounds.len(), ThemeName::ALL.len());
    }
}

/// Themes of your own: `themes/<name>.json` beside settings.json, made from one of Null's
/// (`"based_on"`) with the colours given changed:
///
/// ```json
/// { "based_on": "null", "colors": { "caret": "#7fd1ff", "keyword": "#c792ea" } }
/// ```
pub mod own {
    use super::{Theme, ThemeName};
    use gpui::Hsla;
    use std::path::PathBuf;

    /// What a theme file may set, in the order a new one lists them.
    pub const COLOURS: [&str; 31] = [
        "background",
        "surface",
        "raised",
        "sunken",
        "hairline",
        "line_strong",
        "foreground",
        "muted",
        "faint",
        "caret",
        "accent_soft",
        "on_accent",
        "selection",
        "find_match",
        "current_line",
        "scrim",
        "error",
        "warning",
        "git_added",
        "git_modified",
        "git_deleted",
        "keyword",
        "type",
        "function",
        "macro",
        "string",
        "number",
        "comment",
        "punctuation",
        "attribute",
        "property",
    ];

    /// Where themes of your own are kept.
    pub fn folder() -> Option<PathBuf> {
        Some(crate::settings::Settings::path()?.parent()?.join("themes"))
    }

    /// The themes of your own, by name, in order.
    pub fn names() -> Vec<String> {
        let Some(entries) = folder().and_then(|f| std::fs::read_dir(f).ok()) else { return Vec::new() };
        let mut names: Vec<String> = entries
            .filter_map(|e| {
                let path = e.ok()?.path();
                if path.extension()? != "json" {
                    return None;
                }
                Some(path.file_stem()?.to_string_lossy().into_owned())
            })
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    /// `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub fn colour(text: &str) -> Option<Hsla> {
        let hex = text.trim().strip_prefix('#')?;
        let hex = match hex.len() {
            3 => hex.chars().flat_map(|c| [c, c]).collect::<String>() + "ff",
            6 => format!("{hex}ff"),
            8 => hex.to_string(),
            _ => return None,
        };
        Some(gpui::rgba(u32::from_str_radix(&hex, 16).ok()?).into())
    }

    fn hex(colour: Hsla) -> String {
        let c = gpui::Rgba::from(colour);
        let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        let (r, g, b, a) = (byte(c.r), byte(c.g), byte(c.b), byte(c.a));
        if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
    }

    /// A theme from its file's text: the one it's based on, with its colours over it. What
    /// it says that can't be read is skipped and told.
    pub fn read(text: &str) -> Result<(Theme, Vec<String>), String> {
        let json: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("it isn't JSON ({e})"))?;
        // Named as the theme lists show it ("Paper"), or as settings.json has it ("paper").
        let base = match json.get("based_on") {
            None => ThemeName::Null,
            Some(b) => {
                let name = b.as_str().map(str::to_lowercase).unwrap_or_default();
                serde_json::from_value(name.into()).map_err(|_| format!("there's no theme called {b}"))?
            }
        };
        let mut theme = Theme::named(base);
        let mut problems: Vec<String> = json
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, _)| key)
            .filter(|key| !matches!(key.as_str(), "based_on" | "colors"))
            .map(|key| format!("\"{key}\" isn't read (colours go in \"colors\")"))
            .collect();
        for (name, value) in json.get("colors").and_then(|c| c.as_object()).into_iter().flatten() {
            if name == "ansi" {
                let colours: Vec<Option<Hsla>> =
                    value.as_array().into_iter().flatten().map(|v| v.as_str().and_then(colour)).collect();
                match <[Option<Hsla>; 16]>::try_from(colours) {
                    Ok(all) if all.iter().all(Option::is_some) => theme.ansi = all.map(|c| c.unwrap_or_default()),
                    _ => problems.push("\"ansi\" takes 16 colours".to_string()),
                }
                continue;
            }
            match (theme.slot(name), value.as_str().and_then(colour)) {
                (Some(slot), Some(c)) => *slot = c,
                (None, _) => problems.push(format!("no colour called \"{name}\"")),
                (_, None) => problems.push(format!("\"{name}\" isn't a colour like \"#ff8800\"")),
            }
        }
        Ok((theme, problems))
    }

    /// The theme of your own called `name`, and what in it couldn't be read.
    pub fn load(name: &str) -> Result<(Theme, Vec<String>), String> {
        // A name, not a way out of the themes folder.
        if name.contains(['/', '\\']) || name.starts_with('.') {
            return Err(format!("\"{name}\" isn't a theme's name"));
        }
        let path = folder().ok_or("no settings folder")?.join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path).map_err(|_| format!("there's no theme file {name}.json"))?;
        read(&text)
    }

    /// A new theme file: `theme`'s every colour (made from `base`), written out to change.
    pub fn starter(base: ThemeName, theme: &Theme) -> String {
        let mut theme = theme.clone();
        let mut colours = serde_json::Map::new();
        for name in COLOURS {
            if let Some(c) = theme.slot(name) {
                colours.insert(name.to_string(), hex(*c).into());
            }
        }
        colours.insert("ansi".into(), theme.ansi.iter().map(|c| serde_json::Value::from(hex(*c))).collect());
        let based_on = serde_json::to_value(base).unwrap_or_default();
        let file = serde_json::json!({ "based_on": based_on, "colors": colours });
        serde_json::to_string_pretty(&file).unwrap_or_default() + "\n"
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_theme_of_your_own_changes_what_it_says() {
            let (theme, problems) = read(
                r##"{ "based_on": "paper", "colors": { "caret": "#ff0000", "nope": "#fff", "keyword": "red" } }"##,
            )
            .unwrap();
            assert_eq!(theme.caret, colour("#ff0000").unwrap());
            assert_eq!(theme.background, Theme::named(ThemeName::Paper).background, "the rest is Paper's");
            assert_eq!(problems.len(), 2, "{problems:?}");
            assert!(read("{").is_err());
            assert!(read(r#"{ "based_on": "neon" }"#).is_err());
            // As the lists show it; a key misspelt is said.
            let (moss, problems) = read(r#"{ "based_on": "Moss", "colours": {} }"#).unwrap();
            assert_eq!(moss.background, Theme::named(ThemeName::Moss).background);
            assert_eq!(problems.len(), 1, "{problems:?}");
            assert!(load("../../x").is_err());
            // A new file reads back as the theme it was made from.
            let (again, problems) = read(&starter(ThemeName::Moss, &Theme::named(ThemeName::Moss))).unwrap();
            assert!(problems.is_empty(), "{problems:?}");
            assert_eq!(hex(again.keyword), hex(Theme::named(ThemeName::Moss).keyword));
            assert_eq!(hex(again.ansi[9]), hex(Theme::named(ThemeName::Moss).ansi[9]));
            assert_eq!(colour("#abc"), colour("#aabbcc"));
        }
    }
}
