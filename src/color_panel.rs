//! The Mac's own color panel, to pick a color written in the code: shown set to that color,
//! then asked what it's set to while it stays open. It hides while Null is in the background,
//! as the Mac's panels do, and the color being picked waits for it.

use gpui::Rgba;

/// Shows the panel, set to `color`.
pub fn open(color: Rgba) {
    #[cfg(target_os = "macos")]
    mac::open(color);
    #[cfg(not(target_os = "macos"))]
    let _ = color;
}

/// What the panel shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Panel {
    /// Open, on this color.
    Open(Rgba),
    /// Hidden while Null is in the background (the Mac hides panels then); back with it.
    Away,
    /// Closed (or there's no panel).
    Closed,
}

/// The panel now.
pub fn state() -> Panel {
    #[cfg(target_os = "macos")]
    return mac::state();
    #[cfg(not(target_os = "macos"))]
    Panel::Closed
}

#[cfg(target_os = "macos")]
mod mac {
    use super::Panel;
    use gpui::Rgba;
    use objc2_app_kit::{NSApplication, NSColor, NSColorPanel, NSColorSpace};
    use objc2_foundation::MainThreadMarker;

    pub fn open(color: Rgba) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let panel = NSColorPanel::sharedColorPanel(mtm);
        panel.setShowsAlpha(true);
        let (r, g, b, a) = (color.r as f64, color.g as f64, color.b as f64, color.a as f64);
        panel.setColor(&NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a));
        panel.orderFront(None);
    }

    pub fn state() -> Panel {
        let Some(mtm) = MainThreadMarker::new() else { return Panel::Closed };
        if !NSColorPanel::sharedColorPanelExists(mtm) {
            return Panel::Closed;
        }
        let panel = NSColorPanel::sharedColorPanel(mtm);
        if !panel.isVisible() {
            let active = NSApplication::sharedApplication(mtm).isActive();
            return if active { Panel::Closed } else { Panel::Away };
        }
        match panel.color().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()) {
            Some(color) => Panel::Open(Rgba {
                r: color.redComponent() as f32,
                g: color.greenComponent() as f32,
                b: color.blueComponent() as f32,
                a: color.alphaComponent() as f32,
            }),
            None => Panel::Away,
        }
    }
}
