//! A small floating field for pasting an API key. The key goes to the system keychain.

use crate::ai::{self, ProviderId};
use crate::text_input::TextInput;
use crate::theme::Theme;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, Window, actions, div, prelude::*, px,
};

actions!(key_prompt, [SaveKey, CancelKey]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("KeyPrompt");
    cx.bind_keys([KeyBinding::new("enter", SaveKey, ctx), KeyBinding::new("escape", CancelKey, ctx)]);
}

pub enum KeyPromptEvent {
    /// Done; carries a message to show (saved, removed, or what went wrong).
    Finished(String),
    Cancelled,
}

pub struct KeyPrompt {
    provider: ProviderId,
    input: Entity<TextInput>,
}

impl EventEmitter<KeyPromptEvent> for KeyPrompt {}

impl KeyPrompt {
    pub fn new(provider: ProviderId, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Paste the API key", cx);
            input.masked = true;
            input
        });
        Self { provider, input }
    }

    fn save(&mut self, _: &SaveKey, _: &mut Window, cx: &mut Context<Self>) {
        let key = self.input.read(cx).text().to_string();
        let label = self.provider.label();
        let message = match ai::store_api_key(self.provider, &key) {
            Ok(()) if key.trim().is_empty() => format!("Removed the {label} key"),
            Ok(()) => format!("Saved the {label} key in the keychain"),
            Err(err) => format!("Couldn't save the key: {err}"),
        };
        cx.emit(KeyPromptEvent::Finished(message));
    }

    fn cancel(&mut self, _: &CancelKey, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(KeyPromptEvent::Cancelled);
    }
}

impl Focusable for KeyPrompt {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for KeyPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let store = if cfg!(target_os = "macos") {
            "macOS Keychain"
        } else if cfg!(target_os = "windows") {
            "Windows Credential Manager"
        } else {
            "system keyring"
        };
        div()
            .key_context("KeyPrompt")
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(480.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .rounded(px(14.))
            .border_1()
            .border_color(theme.hairline)
            .bg(theme.raised)
            .shadow_lg()
            .text_size(px(13.))
            .child(div().text_size(px(14.)).text_color(theme.foreground).child(format!("{} API key", self.provider.label())))
            .child(
                div()
                    .h(px(32.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(7.))
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.hairline)
                    .line_height(px(20.))
                    .child(self.input.clone()),
            )
            .child(div().text_size(px(12.)).text_color(theme.faint).child(format!(
                "Stored in your {store}, never in the settings file. Leave it empty and press ↵ to remove the saved key."
            )))
    }
}
