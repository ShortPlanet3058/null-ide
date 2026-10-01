use gpui::{Global, Hsla, rgb, rgba};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    Oled,
    Graphite,
    Paper,
}

impl ThemeName {
    pub fn label(self) -> &'static str {
        match self {
            ThemeName::Oled => "OLED",
            ThemeName::Graphite => "Graphite",
            ThemeName::Paper => "Paper",
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
    /// Dims what is behind a floating layer.
    pub scrim: Hsla,
    pub selection: Hsla,
    /// Other occurrences of the search, behind the current one.
    pub find_match: Hsla,
    pub error: Hsla,
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
            ThemeName::Oled => Self::oled(),
            ThemeName::Graphite => Self::graphite(),
            ThemeName::Paper => Self::paper(),
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
            scrim: rgba(0x00000073).into(),
            selection: rgba(0xf2b35b38).into(),
            find_match: rgba(0xffffff1f).into(),
            error: rgb(0xe88d8d).into(),
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
            scrim: rgba(0x00000052).into(),
            selection: rgba(0xefb56533).into(),
            find_match: rgba(0xffffff1f).into(),
            error: rgb(0xea9393).into(),
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
            scrim: rgba(0x14141929).into(),
            selection: rgba(0xb4620c2b).into(),
            find_match: rgba(0x1d1e221a).into(),
            error: rgb(0xb23a3a).into(),
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
        }
    }
}
