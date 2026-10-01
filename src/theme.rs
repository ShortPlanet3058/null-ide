use gpui::{App, Global, Hsla, rgb, rgba};

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

pub fn init(cx: &mut App) {
    cx.set_global(Theme::oled());
}
