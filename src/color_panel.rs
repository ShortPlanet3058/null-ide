//! The Mac's own color panel, to pick a color written in the code: shown set to that color,
//! then asked what it's set to while it stays open.

use gpui::Rgba;

/// Shows the panel, set to `color`.
pub fn open(color: Rgba) {
    #[cfg(target_os = "macos")]
    mac::open(color);
    #[cfg(not(target_os = "macos"))]
    let _ = color;
}

/// The panel's color while it's open; None once it's closed (or there's no panel).
pub fn color() -> Option<Rgba> {
    #[cfg(target_os = "macos")]
    return mac::color();
    #[cfg(not(target_os = "macos"))]
    None
}

#[cfg(target_os = "macos")]
mod mac {
    use gpui::Rgba;
    use objc2_app_kit::{NSColor, NSColorPanel, NSColorSpace};
    use objc2_foundation::MainThreadMarker;

    pub fn open(color: Rgba) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let panel = NSColorPanel::sharedColorPanel(mtm);
        panel.setShowsAlpha(true);
        let (r, g, b, a) = (color.r as f64, color.g as f64, color.b as f64, color.a as f64);
        panel.setColor(&NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a));
        panel.orderFront(None);
    }

    pub fn color() -> Option<Rgba> {
        let mtm = MainThreadMarker::new()?;
        if !NSColorPanel::sharedColorPanelExists(mtm) {
            return None;
        }
        let panel = NSColorPanel::sharedColorPanel(mtm);
        if !panel.isVisible() {
            return None;
        }
        let color = panel.color().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        Some(Rgba {
            r: color.redComponent() as f32,
            g: color.greenComponent() as f32,
            b: color.blueComponent() as f32,
            a: color.alphaComponent() as f32,
        })
    }
}
