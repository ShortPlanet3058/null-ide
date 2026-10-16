//! Null's questions in a dialog ("Save changes?"). On Windows the window behind GPUI's
//! dialog was seen taking keys: it's disabled here until the question is answered, as
//! Windows' own modal dialogs do (and enabled again even when the answer isn't waited for).
//! Elsewhere it's the platform's dialog as it is.

use gpui::{App, PromptButton, PromptLevel, Window};
use std::future::Future;

/// `window.prompt`, with the window kept from taking input while it's open (Windows).
pub fn ask<T>(
    window: &mut Window,
    level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    answers: &[T],
    cx: &mut App,
) -> impl Future<Output = Result<usize, futures::channel::oneshot::Canceled>> + use<T>
where
    T: Clone + Into<PromptButton>,
{
    #[cfg(all(windows, not(test)))]
    let held = windows::Held::new(window);
    let answer = window.prompt(level, message, detail, answers, cx);
    async move {
        let answered = answer.await;
        #[cfg(all(windows, not(test)))]
        drop(held);
        answered
    }
}

// (Not in tests: GPUI's test windows have no handle to give.)
#[cfg(all(windows, not(test)))]
mod windows {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnableWindow(hwnd: isize, enable: i32) -> i32;
    }

    /// Dialogs open over each window: it's enabled again when the last one closes.
    static OPEN: Mutex<Option<HashMap<isize, usize>>> = Mutex::new(None);

    /// A window disabled while a dialog is over it; enabled again when dropped.
    pub struct Held(Option<isize>);

    impl Held {
        pub fn new(window: &gpui::Window) -> Self {
            let hwnd = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
                _ => return Held(None),
            };
            if let Ok(mut open) = OPEN.lock() {
                let count = open.get_or_insert_with(HashMap::new).entry(hwnd).or_insert(0);
                *count += 1;
            }
            // SAFETY: a window handle GPUI gave for a window that's open; no pointers.
            unsafe { EnableWindow(hwnd, 0) };
            Held(Some(hwnd))
        }
    }

    impl Drop for Held {
        fn drop(&mut self) {
            let Some(hwnd) = self.0 else { return };
            let last = OPEN.lock().ok().and_then(|mut open| {
                let count = open.as_mut()?.get_mut(&hwnd)?;
                *count -= 1;
                Some(*count == 0)
            });
            if last != Some(false) {
                // SAFETY: as above; a closed window's handle is simply refused.
                unsafe { EnableWindow(hwnd, 1) };
            }
        }
    }
}
