use crate::editor::{Editor, EditorEvent};
use crate::menus::{self, ToggleFadeWhileTyping};
use crate::theme::Theme;
use gpui::{
    Context, Entity, MouseMoveEvent, Pixels, Point, Subscription, Window, WindowControlArea, div, prelude::*, px,
};
use std::time::{Duration, Instant};

/// Chrome dims to this while you type.
const DIMMED: f32 = 0.12;
const FADE_OUT: Duration = Duration::from_millis(700);
const FADE_IN: Duration = Duration::from_millis(180);
/// Mouse movement smaller than this (trackpad jitter) doesn't bring the chrome back.
const WAKE_DISTANCE: f32 = 6.;

/// Opacity of the title and status bars. When fading is on, they recede while
/// you type and come back as soon as you reach for the mouse.
struct ChromeFade {
    visible: bool,
    from: f32,
    changed_at: Instant,
}

impl ChromeFade {
    fn opacity(&self, now: Instant) -> (f32, bool) {
        let (target, duration) = if self.visible { (1., FADE_IN) } else { (DIMMED, FADE_OUT) };
        let t = ((now - self.changed_at).as_secs_f32() / duration.as_secs_f32()).min(1.);
        let eased = t * t * (3. - 2. * t);
        (self.from + (target - self.from) * eased, t < 1.)
    }

    fn set_visible(&mut self, visible: bool) {
        if visible != self.visible {
            let now = Instant::now();
            self.from = self.opacity(now).0;
            self.visible = visible;
            self.changed_at = now;
        }
    }
}

pub struct Workspace {
    editor: Entity<Editor>,
    /// Off by default: some people want the file name and caret position visible at all times.
    fade_while_typing: bool,
    chrome: ChromeFade,
    last_mouse: Option<Point<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(editor: Entity<Editor>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&editor, |_, _, cx| cx.notify()),
            cx.subscribe(&editor, |this, _, event, cx| match event {
                EditorEvent::Edited if this.fade_while_typing => {
                    this.chrome.set_visible(false);
                    cx.notify();
                }
                EditorEvent::Edited => {}
            }),
        ];
        Self {
            editor,
            fade_while_typing: false,
            chrome: ChromeFade { visible: true, from: 1., changed_at: Instant::now() },
            last_mouse: None,
            _subscriptions: subscriptions,
        }
    }

    fn toggle_fade_while_typing(&mut self, _: &ToggleFadeWhileTyping, _: &mut Window, cx: &mut Context<Self>) {
        self.fade_while_typing = !self.fade_while_typing;
        if !self.fade_while_typing {
            self.chrome.set_visible(true);
        }
        menus::set(cx, self.fade_while_typing);
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let moved = self.last_mouse.is_none_or(|last| {
            let d = event.position - last;
            f32::from(d.x).abs() + f32::from(d.y).abs() > WAKE_DISTANCE
        });
        if moved {
            self.last_mouse = Some(event.position);
            if !self.chrome.visible {
                self.chrome.set_visible(true);
                cx.notify();
            }
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let (opacity, fading) = self.chrome.opacity(Instant::now());
        if fading {
            window.request_animation_frame();
        }

        let editor = self.editor.read(cx);
        let (line, col) = editor.caret_point();
        let title = format!("{}{}", editor.file_name(), if editor.buffer.is_dirty() { "  •" } else { "" });
        let path = editor.path().map(|p| p.display().to_string()).unwrap_or_else(|| "untitled".into());
        let language = editor.language_name();

        let titlebar = div()
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .border_b_1()
            .border_color(theme.hairline)
            .bg(theme.surface)
            .text_size(px(12.))
            .text_color(theme.muted)
            .opacity(opacity)
            .window_control_area(WindowControlArea::Drag)
            .child(title);

        let status = div()
            .h(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(16.))
            .border_t_1()
            .border_color(theme.hairline)
            .bg(theme.surface)
            .text_size(px(12.))
            .text_color(theme.muted)
            .opacity(opacity)
            .child(div().flex_1().overflow_hidden().child(path))
            .child(format!("Ln {}, Col {}", line + 1, col + 1))
            .child("Spaces: 4")
            .child(language);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(".SystemUIFont")
            .on_action(cx.listener(Self::toggle_fade_while_typing))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(titlebar)
            .child(div().flex_1().min_h_0().child(self.editor.clone()))
            .child(status)
    }
}
