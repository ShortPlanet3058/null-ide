use crate::editor::Editor;
use crate::fonts::Fonts;
use crate::search::{MAX_MATCHES, SearchQuery};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use crate::ui;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, KeyBinding, SharedString, Subscription,
    Transformation, WeakEntity, Window, actions, div, prelude::*, px, radians, svg,
};

actions!(
    find,
    [
        DeployFind,
        DeployReplace,
        FindNext,
        FindPrevious,
        CloseFind,
        ToggleCaseSensitive,
        ToggleWholeWord,
        ToggleRegex,
        ReplaceNext,
        ReplaceAll,
        OlderSearch,
        NewerSearch,
        UseSelectionForFind,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let editor = Some("Editor");
    let bar = Some("FindBar");
    let replace = Some("ReplaceInput");
    let mut keys = vec![
        KeyBinding::new("secondary-f", DeployFind, editor),
        KeyBinding::new("secondary-g", FindNext, editor),
        KeyBinding::new("secondary-shift-g", FindPrevious, editor),
        KeyBinding::new("escape", CloseFind, editor),
        KeyBinding::new("cmd-e", UseSelectionForFind, editor),
        KeyBinding::new("secondary-f", DeployFind, bar),
        KeyBinding::new("enter", FindNext, bar),
        KeyBinding::new("secondary-g", FindNext, bar),
        KeyBinding::new("shift-enter", FindPrevious, bar),
        KeyBinding::new("secondary-shift-g", FindPrevious, bar),
        KeyBinding::new("escape", CloseFind, bar),
        KeyBinding::new("up", OlderSearch, bar),
        KeyBinding::new("down", NewerSearch, bar),
        KeyBinding::new("enter", ReplaceNext, replace),
        KeyBinding::new("secondary-enter", ReplaceAll, replace),
    ];
    if cfg!(target_os = "macos") {
        keys.extend([
            KeyBinding::new("alt-cmd-f", DeployReplace, editor),
            KeyBinding::new("alt-cmd-f", DeployReplace, bar),
            KeyBinding::new("alt-cmd-c", ToggleCaseSensitive, bar),
            KeyBinding::new("alt-cmd-w", ToggleWholeWord, bar),
            KeyBinding::new("alt-cmd-r", ToggleRegex, bar),
        ]);
    } else {
        keys.extend([
            KeyBinding::new("ctrl-h", DeployReplace, editor),
            KeyBinding::new("ctrl-h", DeployReplace, bar),
            KeyBinding::new("alt-c", ToggleCaseSensitive, bar),
            KeyBinding::new("alt-w", ToggleWholeWord, bar),
            KeyBinding::new("alt-r", ToggleRegex, bar),
        ]);
    }
    cx.bind_keys(keys);
}

/// What was searched for, oldest first, in any editor since Null started: ↑ and ↓ in the
/// find field go back through it.
#[derive(Default)]
struct SearchHistory(Vec<String>);

impl gpui::Global for SearchHistory {}

const HISTORY_LEN: usize = 50;

/// What was searched for last, in any file.
pub fn latest_search(cx: &App) -> Option<String> {
    cx.try_global::<SearchHistory>().and_then(|h| h.0.last().cloned())
}

/// `text` kept as the latest search, for ↑ in the find field.
pub fn remember_search(text: &str, cx: &mut App) {
    remember(&mut cx.default_global::<SearchHistory>().0, text);
}

/// Keeps `text` as the latest search (once, however often it's searched).
fn remember(history: &mut Vec<String>, text: &str) {
    if text.is_empty() {
        return;
    }
    history.retain(|t| t != text);
    history.push(text.to_string());
    if history.len() > HISTORY_LEN {
        history.remove(0);
    }
}

const CHEVRON: &str = "icons/chevron-right.svg";
const CLOSE: &str = "icons/x.svg";
const WIDTH: f32 = 460.;
const CONTROL: f32 = 26.;

/// The find (and replace) bar that floats over the top right of an editor.
pub struct FindBar {
    editor: WeakEntity<Editor>,
    find: Entity<TextInput>,
    replace: Entity<TextInput>,
    show_replace: bool,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    /// While going through earlier searches with ↑ ↓: which one shows, and what was typed
    /// before, to come back to.
    recalled: Option<(usize, String)>,
    _subscription: Subscription,
}

impl FindBar {
    pub fn new(editor: WeakEntity<Editor>, query: &SearchQuery, show_replace: bool, cx: &mut Context<Self>) -> Self {
        let find = cx.new(|cx| {
            let mut input = TextInput::new("Find", cx);
            input.set_text(&query.text, cx);
            input
        });
        let replace = cx.new(|cx| TextInput::new("Replace", cx));
        let subscription = cx.subscribe(&find, |this, input, TextInputEvent::Changed, cx| {
            // Typing (rather than a search recalled) starts again from the newest.
            let history = cx.try_global::<SearchHistory>().map(|h| &h.0);
            let shown = this.recalled.as_ref().and_then(|(at, _)| history?.get(*at));
            if shown.map(String::as_str) != Some(input.read(cx).text()) {
                this.recalled = None;
            }
            this.search(cx)
        });
        Self {
            editor,
            find,
            replace,
            show_replace,
            case_sensitive: query.case_sensitive,
            whole_word: query.whole_word,
            regex: query.regex,
            recalled: None,
            _subscription: subscription,
        }
    }

    pub fn query(&self, cx: &App) -> SearchQuery {
        SearchQuery {
            text: self.find.read(cx).text().to_string(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
        }
    }

    /// Shows the bar again, optionally with new text, focused on the right field.
    pub fn show(&mut self, text: Option<String>, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = text {
            self.find.update(cx, |input, cx| input.set_text(&text, cx));
        } else {
            self.find.update(cx, |input, cx| input.select_all_text(cx));
        }
        self.show_replace |= replace;
        let target = if replace { &self.replace } else { &self.find };
        window.focus(&target.focus_handle(cx));
        cx.notify();
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.editor.update(cx, |editor, cx| editor.set_search(query, cx)).ok();
        cx.notify();
    }

    fn deploy_find(&mut self, _: &DeployFind, window: &mut Window, cx: &mut Context<Self>) {
        self.show(None, false, window, cx);
    }

    fn deploy_replace(&mut self, _: &DeployReplace, window: &mut Window, cx: &mut Context<Self>) {
        self.show(None, true, window, cx);
    }

    fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.remember_search(cx);
        self.editor.update(cx, |editor, cx| editor.select_next_match(cx)).ok();
    }

    fn find_previous(&mut self, _: &FindPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.remember_search(cx);
        self.editor.update(cx, |editor, cx| editor.select_previous_match(cx)).ok();
    }

    fn close(&mut self, _: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_search(cx);
        self.editor.update(cx, |editor, cx| editor.close_find(window, cx)).ok();
    }

    /// What the find field holds becomes the latest search, unless it's one being recalled.
    fn remember_search(&mut self, cx: &mut Context<Self>) {
        let text = self.find.read(cx).text().to_string();
        remember(&mut cx.default_global::<SearchHistory>().0, &text);
        self.recalled = None;
    }

    /// ↑: the search before the one showing (skipping what's in the field already).
    fn older_search(&mut self, _: &OlderSearch, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find.focus_handle(cx).is_focused(window) {
            return;
        }
        let history = cx.try_global::<SearchHistory>().map(|h| h.0.clone()).unwrap_or_default();
        let typed = self.find.read(cx).text().to_string();
        let (from, draft) = match self.recalled.take() {
            Some((at, draft)) => (at, draft),
            None => (history.len(), typed.clone()),
        };
        let older = (0..from).rev().find(|&i| history[i] != typed);
        match older {
            Some(at) => self.recall(at, &history[at].clone(), draft, cx),
            None => self.recalled = (from < history.len()).then_some((from, draft)),
        }
    }

    /// ↓: the search after the one showing, then what was typed before going back.
    fn newer_search(&mut self, _: &NewerSearch, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find.focus_handle(cx).is_focused(window) {
            return;
        }
        let Some((at, draft)) = self.recalled.take() else { return };
        let history = cx.try_global::<SearchHistory>().map(|h| h.0.clone()).unwrap_or_default();
        match history.get(at + 1) {
            Some(newer) => self.recall(at + 1, &newer.clone(), draft, cx),
            None => self.find.update(cx, |input, cx| input.set_text(&draft, cx)),
        }
    }

    fn recall(&mut self, at: usize, text: &str, draft: String, cx: &mut Context<Self>) {
        self.find.update(cx, |input, cx| input.set_text(text, cx));
        self.recalled = Some((at, draft));
    }

    fn toggle_case_sensitive(&mut self, _: &ToggleCaseSensitive, _: &mut Window, cx: &mut Context<Self>) {
        self.case_sensitive = !self.case_sensitive;
        self.search(cx);
    }

    fn toggle_whole_word(&mut self, _: &ToggleWholeWord, _: &mut Window, cx: &mut Context<Self>) {
        self.whole_word = !self.whole_word;
        self.search(cx);
    }

    fn toggle_regex(&mut self, _: &ToggleRegex, _: &mut Window, cx: &mut Context<Self>) {
        self.regex = !self.regex;
        self.search(cx);
    }

    fn replace_next(&mut self, _: &ReplaceNext, _: &mut Window, cx: &mut Context<Self>) {
        let replacement = self.replace.read(cx).text().to_string();
        self.editor.update(cx, |editor, cx| editor.replace_next_match(&replacement, cx)).ok();
    }

    fn replace_all(&mut self, _: &ReplaceAll, _: &mut Window, cx: &mut Context<Self>) {
        let replacement = self.replace.read(cx).text().to_string();
        self.editor.update(cx, |editor, cx| editor.replace_all_matches(&replacement, cx)).ok();
    }

    fn icon_button(
        id: &'static str,
        icon: &'static str,
        turn: f32,
        theme: &Theme,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .debug_selector(|| format!("find {id}"))
            .size(px(CONTROL))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(ui::R_ROW))
            .cursor_pointer()
            .group(id)
            .hover(|s| s.bg(theme.hairline))
            // An icon takes its own colour (it doesn't inherit the text's).
            .child(
                svg()
                    .path(icon)
                    .size(px(14.))
                    .text_color(theme.muted)
                    .group_hover(id, |s| s.text_color(theme.foreground))
                    .with_transformation(Transformation::rotate(radians(turn))),
            )
            .active(|s| s.opacity(0.7))
            .tooltip({
                let (label, action) = Self::tooltip_for(id);
                ui::tip(label, action)
            })
            .on_click(on_click)
    }

    /// What each button is called, and the action whose keys it shows.
    fn tooltip_for(id: &str) -> (&'static str, Option<Box<dyn gpui::Action>>) {
        match id {
            "toggle-replace" => ("Replace", Some(Box::new(DeployReplace))),
            "case" => ("Match case", Some(Box::new(ToggleCaseSensitive))),
            "word" => ("Whole word", Some(Box::new(ToggleWholeWord))),
            "regex" => ("Regular expression", Some(Box::new(ToggleRegex))),
            "previous" => ("Previous match", Some(Box::new(FindPrevious))),
            "next" => ("Next match", Some(Box::new(FindNext))),
            "close" => ("Close", Some(Box::new(CloseFind))),
            _ => ("", None),
        }
    }

    fn toggle(
        id: &'static str,
        label: &'static str,
        on: bool,
        theme: &Theme,
        font: SharedString,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .debug_selector(|| format!("find {id}"))
            .h(px(CONTROL))
            .min_w(px(CONTROL))
            .px(px(4.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(ui::R_ROW))
            .font_family(font)
            .text_size(px(ui::T_SM))
            .text_color(if on { theme.caret } else { theme.muted })
            .when(on, |b| b.bg(theme.accent_soft))
            .when(!on, |b| b.hover(|s| s.bg(theme.hairline).text_color(theme.foreground)))
            .child(label)
            .tooltip({
                let (label, action) = Self::tooltip_for(id);
                ui::tip(label, action)
            })
            .active(|s| s.opacity(0.7))
            .on_click(on_click)
    }

    /// A field, with something quiet at its right end (the match count) when given.
    fn field(input: Entity<TextInput>, invalid: bool, theme: &Theme, trailing: Option<AnyElement>) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .h(px(ui::FIELD))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(ui::R_ROW))
            .bg(theme.background)
            .border_1()
            .border_color(if invalid { theme.error } else { theme.hairline })
            .overflow_hidden()
            .child(div().flex_1().min_w_0().overflow_hidden().child(input))
            .children(trailing)
    }
}

impl Focusable for FindBar {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.find.focus_handle(cx)
    }
}

impl Render for FindBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let code_font = cx.global::<Fonts>().code.clone();
        let (invalid, status): (bool, SharedString) = self
            .editor
            .upgrade()
            .and_then(|editor| {
                let search = editor.read(cx).search.as_ref()?;
                Some(if search.is_invalid() {
                    (true, "Invalid pattern".into())
                } else if search.query.text.is_empty() {
                    (false, "".into())
                } else if search.matches.is_empty() {
                    (false, "No results".into())
                } else {
                    let total = search.matches.len();
                    let total = if total >= MAX_MATCHES { format!("{total}+") } else { total.to_string() };
                    let current = search.current.map_or("?".to_string(), |i| (i + 1).to_string());
                    (false, format!("{current} of {total}").into())
                })
            })
            .unwrap_or((false, "".into()));

        let this = cx.entity().downgrade();
        let act = move |f: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            let this = this.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| f(this, window, cx)).ok();
            }
        };

        let find_row = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(Self::icon_button(
                "toggle-replace",
                CHEVRON,
                if self.show_replace { std::f32::consts::FRAC_PI_2 } else { 0. },
                &theme,
                act(|this, window, cx| {
                    this.show_replace = !this.show_replace;
                    let target = if this.show_replace { &this.replace } else { &this.find };
                    window.focus(&target.focus_handle(cx));
                    cx.notify();
                }),
            ))
            .child(Self::field(
                self.find.clone(),
                invalid,
                &theme,
                (!status.is_empty()).then(|| {
                    div()
                        .flex_none()
                        .text_size(px(ui::T_SM))
                        .text_color(if invalid { theme.error } else { theme.muted })
                        .child(status.clone())
                        .into_any_element()
                }),
            ))
            .child(Self::toggle(
                "case",
                "Aa",
                self.case_sensitive,
                &theme,
                code_font.clone(),
                act(|this, window, cx| this.toggle_case_sensitive(&ToggleCaseSensitive, window, cx)),
            ))
            .child(Self::toggle(
                "word",
                "W",
                self.whole_word,
                &theme,
                code_font.clone(),
                act(|this, window, cx| this.toggle_whole_word(&ToggleWholeWord, window, cx)),
            ))
            .child(Self::toggle(
                "regex",
                ".*",
                self.regex,
                &theme,
                code_font.clone(),
                act(|this, window, cx| this.toggle_regex(&ToggleRegex, window, cx)),
            ))
            .child(Self::icon_button(
                "previous",
                CHEVRON,
                -std::f32::consts::FRAC_PI_2,
                &theme,
                act(|this, window, cx| this.find_previous(&FindPrevious, window, cx)),
            ))
            .child(Self::icon_button(
                "next",
                CHEVRON,
                std::f32::consts::FRAC_PI_2,
                &theme,
                act(|this, window, cx| this.find_next(&FindNext, window, cx)),
            ))
            .child(Self::icon_button(
                "close",
                CLOSE,
                0.,
                &theme,
                act(|this, window, cx| this.close(&CloseFind, window, cx)),
            ));

        let text_button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .h(px(CONTROL))
                .px(px(10.))
                .flex_none()
                .flex()
                .items_center()
                .rounded(px(ui::R_ROW))
                .text_size(px(ui::T_SM))
                .text_color(theme.muted)
                .hover(|s| s.bg(theme.hairline).text_color(theme.foreground))
                .child(label)
        };
        let replace_row = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(div().w(px(CONTROL)).flex_none())
            .child(div().key_context("ReplaceInput").flex_1().min_w_0().flex().child(Self::field(
                self.replace.clone(),
                false,
                &theme,
                None,
            )))
            .child(
                text_button("replace-one", "Replace")
                    .active(|s| s.opacity(0.7))
                    .on_click(act(|this, window, cx| this.replace_next(&ReplaceNext, window, cx))),
            )
            .child(
                text_button("replace-all", "Replace all")
                    .active(|s| s.opacity(0.7))
                    .on_click(act(|this, window, cx| this.replace_all(&ReplaceAll, window, cx))),
            );

        div()
            .key_context("FindBar")
            .on_action(cx.listener(Self::deploy_find))
            .on_action(cx.listener(Self::deploy_replace))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::toggle_case_sensitive))
            .on_action(cx.listener(Self::toggle_whole_word))
            .on_action(cx.listener(Self::toggle_regex))
            .on_action(cx.listener(Self::replace_next))
            .on_action(cx.listener(Self::replace_all))
            .on_action(cx.listener(Self::older_search))
            .on_action(cx.listener(Self::newer_search))
            .occlude()
            .w(px(WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(6.))
            .rounded(px(ui::R_POPOVER))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.hairline)
            .shadow_md()
            .text_size(px(ui::T_MD))
            .line_height(px(20.))
            .text_color(theme.foreground)
            .child(find_row)
            .when(self.show_replace, |bar| bar.child(replace_row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Selection;

    /// ↑ goes back through what was searched for (in any editor), ↓ comes forward again
    /// and back to what was being typed.
    #[gpui::test]
    fn up_and_down_go_through_earlier_searches(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let (e, cx) = cx.add_window_view(|_, cx| {
            Editor::new(crate::buffer::Buffer::from_text("alpha beta gamma\n"), Some("x.txt".into()), cx)
        });
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            e.selection = Selection { anchor: 0, head: 0 };
        });
        let field = |cx: &mut gpui::VisualTestContext| e.read_with(cx, |e, cx| e.find_query(cx).unwrap().text);
        cx.simulate_keystrokes("cmd-f");
        for word in ["alpha", "beta", "alpha"] {
            cx.simulate_keystrokes("cmd-a");
            cx.simulate_input(word);
            cx.simulate_keystrokes("enter");
        }
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("gam");
        cx.simulate_keystrokes("up");
        assert_eq!(field(cx), "alpha");
        cx.simulate_keystrokes("up");
        assert_eq!(field(cx), "beta");
        // The oldest stays.
        cx.simulate_keystrokes("up");
        assert_eq!(field(cx), "beta");
        cx.simulate_keystrokes("down");
        assert_eq!(field(cx), "alpha");
        cx.simulate_keystrokes("down");
        assert_eq!(field(cx), "gam");
        // Typing over a recalled one starts from the newest again.
        cx.simulate_keystrokes("up");
        cx.simulate_input("x");
        cx.simulate_keystrokes("up");
        assert_eq!(field(cx), "alpha");
    }

    #[test]
    fn a_search_is_kept_once_newest_last() {
        let mut history = Vec::new();
        for text in ["a", "b", "", "a"] {
            remember(&mut history, text);
        }
        assert_eq!(history, ["b", "a"]);
    }
}
