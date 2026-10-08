//! SVGs drawn for the preview. gpui can draw one from its bytes, but its colours come out
//! with red and blue swapped and at 1×; here they're drawn with resvg (gpui's own) at the
//! screen's scale, in the order gpui shows pixels in.

use gpui::RenderImage;
use resvg::{tiny_skia, usvg};
use std::sync::{Arc, LazyLock};

/// The longest side drawn, in pixels: a huge drawing is drawn smaller, not out of memory.
const MAX_PIXELS: f32 = 4096.;

/// The fonts text in an SVG is drawn with: the Mac's, read once.
static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_system_fonts();
    Arc::new(fonts)
});

/// `svg` drawn `scale` times (2 on a Retina screen), and its size in points. None when it
/// isn't an SVG that draws.
pub fn draw(svg: &[u8], scale: f32) -> Option<(Arc<RenderImage>, (f32, f32))> {
    let options = usvg::Options { fontdb: FONTS.clone(), ..Default::default() };
    let tree = usvg::Tree::from_data(svg, &options).ok()?;
    let (width, height) = (tree.size().width(), tree.size().height());
    let scale = scale.min(MAX_PIXELS / width.max(height).max(1.));
    let mut pixmap = tiny_skia::Pixmap::new((width * scale).ceil() as u32, (height * scale).ceil() as u32)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some((Arc::new(RenderImage::new(vec![image::Frame::new(bgra(&pixmap)?)])), (width, height)))
}

/// SVG files drawn lately, by path, with the time they were changed.
#[allow(clippy::type_complexity)]
static DRAWN: LazyLock<std::sync::Mutex<Vec<(std::path::PathBuf, std::time::SystemTime, Arc<RenderImage>)>>> =
    LazyLock::new(Default::default);

/// What to show for an image file: an SVG drawn here (in its colours, sharp on a Retina
/// screen), anything else as gpui shows it.
pub fn source(path: &std::path::Path) -> gpui::ImageSource {
    let svg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let changed = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let (true, Some(changed)) = (svg, changed) else { return path.to_path_buf().into() };
    let mut drawn = DRAWN.lock().unwrap();
    if let Some((_, _, image)) = drawn.iter().find(|(p, t, _)| p == path && *t == changed) {
        return gpui::ImageSource::Render(image.clone());
    }
    let Some((image, _)) = std::fs::read(path).ok().and_then(|bytes| draw(&bytes, 2.)) else {
        return path.to_path_buf().into();
    };
    drawn.retain(|(p, _, _)| p != path);
    if drawn.len() >= 32 {
        drawn.remove(0);
    }
    drawn.push((path.to_path_buf(), changed, image.clone()));
    gpui::ImageSource::Render(image)
}

/// tiny-skia's pixels (red first, alpha multiplied in) in gpui's order: blue first, plain.
fn bgra(pixmap: &tiny_skia::Pixmap) -> Option<image::RgbaImage> {
    let mut data = Vec::with_capacity(pixmap.pixels().len() * 4);
    for pixel in pixmap.pixels() {
        let c = pixel.demultiply();
        data.extend([c.blue(), c.green(), c.red(), c.alpha()]);
    }
    image::RgbaImage::from_raw(pixmap.width(), pixmap.height(), data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svg_is_drawn_in_its_colours() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5"><rect width="10" height="5" fill="#f2a33a"/></svg>"##;
        let (image, size) = draw(svg, 2.).expect("drawn");
        assert_eq!(size, (10., 5.));
        let pixels = image.as_bytes(0).unwrap();
        assert_eq!(pixels.len(), 20 * 10 * 4, "twice as many pixels each way");
        // Blue, green, red, alpha: the orange stays orange.
        assert_eq!(&pixels[..4], &[0x3a, 0xa3, 0xf2, 0xff]);
        assert!(draw(b"<svg", 2.).is_none(), "not an SVG that draws");
    }

    #[test]
    fn svg_files_are_drawn_here_and_others_left_to_gpui() {
        let dir = crate::tools::test_dir("svg-source");
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("a.svg");
        std::fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#).unwrap();
        assert!(matches!(source(&svg), gpui::ImageSource::Render(_)));
        assert!(matches!(source(&dir.join("b.png")), gpui::ImageSource::Resource(_)));
        std::fs::write(&svg, "<svg").unwrap();
        assert!(!matches!(source(&svg), gpui::ImageSource::Render(_)), "broken: left to gpui's fallback");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_huge_svg_is_drawn_smaller() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20000" height="100"/>"#;
        let (image, _) = draw(svg, 2.).expect("drawn");
        assert!(image.size(0).width.0 <= 4096);
    }
}
