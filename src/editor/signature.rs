//! Parameter hints: while typing a call's arguments, the function's signature sits on
//! one line just above the caret, the parameter being typed standing out. It follows
//! the caret and goes away once it leaves the call (or with Esc).

use super::Editor;
use crate::fonts::Fonts;
use crate::theme::Theme;
use crate::ui;
use gpui::{
    AnyElement, Context, Corner, HighlightStyle, StyledText, Task, anchored, deferred, div, point, prelude::*, px,
};
use lsp_types::{ParameterLabel, SignatureHelp};
use std::ops::Range;
use std::time::Duration;

/// Moving or typing inside a call asks again after this pause, to follow the parameter.
const FOLLOW_PAUSE: Duration = Duration::from_millis(60);

pub struct SignatureCard {
    label: String,
    /// The parameter being typed: bytes of `label`.
    active: Option<Range<usize>>,
    /// Other ways to call it (overloads), not shown.
    others: usize,
}

#[derive(Default)]
pub(super) struct Signature {
    pub card: Option<SignatureCard>,
    pub task: Option<Task<()>>,
}

impl SignatureCard {
    /// The signature a server suggests, if it gave one.
    pub fn from_lsp(help: SignatureHelp) -> Option<Self> {
        let index = help.active_signature.unwrap_or(0) as usize;
        let others = help.signatures.len().saturating_sub(1);
        let signature = help.signatures.into_iter().nth(index)?;
        let active = signature.active_parameter.or(help.active_parameter).map(|a| a as usize);
        let label = signature.label;
        let range = active.and_then(|a| signature.parameters?.into_iter().nth(a)).and_then(|p| match p.label {
            ParameterLabel::Simple(name) => label.find(&name).map(|b| b..b + name.len()),
            ParameterLabel::LabelOffsets([from, to]) => {
                let byte = |units: u32| {
                    let mut count = 0;
                    for (b, c) in label.char_indices() {
                        if count >= units {
                            return b;
                        }
                        count += c.len_utf16() as u32;
                    }
                    label.len()
                };
                Some(byte(from)..byte(to))
            }
        });
        Some(Self { label, active: range, others })
    }
}

impl Editor {
    /// After typing: `(` or `,` asks for the signature; inside a call, keep it up to date.
    pub(super) fn signature_after_typing(&mut self, text: &str, cx: &mut Context<Self>) {
        let opens = text.ends_with(['(', ',']);
        if opens || self.signature.card.is_some() {
            self.request_signature(if opens { Duration::ZERO } else { FOLLOW_PAUSE }, cx);
        }
    }

    /// The caret moved: follow the parameter, or close once it leaves the call.
    pub(super) fn signature_after_move(&mut self, cx: &mut Context<Self>) {
        if self.signature.card.is_some() {
            self.request_signature(FOLLOW_PAUSE, cx);
        }
    }

    pub(super) fn close_signature(&mut self, cx: &mut Context<Self>) {
        if self.signature.card.take().is_some() {
            cx.notify();
        }
        self.signature.task = None;
    }

    fn request_signature(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let (Some(lsp), Some(path)) = (self.lsp.clone(), self.path.clone()) else { return };
        if !self.selection.is_empty() || self.multi_cursor() {
            return self.close_signature(cx);
        }
        let position = self.lsp_position(self.selection.head);
        self.signature.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(request) = this.update(cx, |_, cx| lsp.read(cx).signature_help(&path, position)) else { return };
            let help = request.await;
            this.update(cx, |this, cx| {
                // The server says null outside a call: that's when it goes away.
                this.signature.card = help.and_then(SignatureCard::from_lsp);
                cx.notify();
            })
            .ok();
        }));
    }

    /// One quiet line above the caret, in the code's font.
    pub(super) fn render_signature(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = self.signature.card.as_ref()?;
        let bounds = self.caret_bounds(self.selection.head)?;
        let theme = cx.global::<Theme>();
        let strong = HighlightStyle {
            color: Some(theme.foreground),
            font_weight: Some(gpui::FontWeight::SEMIBOLD),
            ..Default::default()
        };
        let text = StyledText::new(card.label.clone()).with_highlights(card.active.clone().map(|r| (r, strong)));
        Some(
            deferred(
                anchored()
                    .anchor(Corner::BottomLeft)
                    .position(point(bounds.left() - px(10.), bounds.top() - px(4.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .occlude()
                            .max_w(px(640.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(10.))
                            .py(px(4.))
                            .rounded(px(ui::R_ROW))
                            .bg(theme.raised)
                            .border_1()
                            .border_color(theme.hairline)
                            .shadow_md()
                            .font_family(cx.global::<Fonts>().code.clone())
                            .text_size(px(ui::T_SM))
                            .text_color(theme.muted)
                            .child(div().min_w_0().truncate().child(text))
                            .when(card.others > 0, |d| {
                                d.child(div().flex_none().text_color(theme.faint).child(format!("+{}", card.others)))
                            }),
                    ),
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{ParameterInformation, SignatureInformation};

    fn help(label: &str, params: Vec<ParameterLabel>, active: u32) -> SignatureHelp {
        SignatureHelp {
            signatures: vec![SignatureInformation {
                label: label.into(),
                documentation: None,
                parameters: Some(
                    params.into_iter().map(|label| ParameterInformation { label, documentation: None }).collect(),
                ),
                active_parameter: None,
            }],
            active_signature: Some(0),
            active_parameter: Some(active),
        }
    }

    #[test]
    fn the_parameter_being_typed_is_found_by_name_or_offsets() {
        let card = SignatureCard::from_lsp(help(
            "fn mix(a: f32, b: f32) -> f32",
            vec![ParameterLabel::Simple("a: f32".into()), ParameterLabel::Simple("b: f32".into())],
            1,
        ))
        .unwrap();
        assert_eq!(&card.label[card.active.unwrap()], "b: f32");
        // Offsets count UTF-16 units: "é" is one, and the byte range still lands on "y".
        let card = SignatureCard::from_lsp(help("é(x, y)", vec![ParameterLabel::LabelOffsets([5, 6])], 0)).unwrap();
        assert_eq!(&card.label[card.active.unwrap()], "y");
    }
}
