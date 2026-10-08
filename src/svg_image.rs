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

/// What an SVG drawn is: the image, and its size in points.
pub type Drawn = (Arc<RenderImage>, (f32, f32));

/// `svg` drawn `scale` times (2 on a Retina screen), and its size in points; files it names
/// are found from `dir` (its folder). None when it isn't an SVG that draws.
pub fn draw(svg: &[u8], scale: f32, dir: Option<&std::path::Path>) -> Option<Drawn> {
    let options =
        usvg::Options { fontdb: FONTS.clone(), resources_dir: dir.map(|d| d.to_path_buf()), ..Default::default() };
    let tree = usvg::Tree::from_data(svg, &options).ok()?;
    let (width, height) = (tree.size().width(), tree.size().height());
    let scale = scale.min(MAX_PIXELS / width.max(height).max(1.));
    let mut pixmap = tiny_skia::Pixmap::new((width * scale).ceil() as u32, (height * scale).ceil() as u32)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some((Arc::new(RenderImage::new(vec![image::Frame::new(bgra(&pixmap)?)])), (width, height)))
}

/// SVG files drawn lately (or found not to draw), by path, with the time they were changed.
#[allow(clippy::type_complexity)]
static DRAWN: LazyLock<std::sync::Mutex<Vec<(std::path::PathBuf, std::time::SystemTime, Option<Drawn>)>>> =
    LazyLock::new(Default::default);

/// An image file shown: an SVG drawn here (in its colours, sharp on a Retina screen, at its
/// own size in points), anything else as gpui shows it.
pub fn image(path: &std::path::Path) -> gpui::Img {
    use gpui::Styled;
    let svg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let changed = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let (true, Some(changed)) = (svg, changed) else { return gpui::img(path.to_path_buf()) };
    let found = DRAWN
        .lock()
        .ok()
        .and_then(|drawn| drawn.iter().find(|(p, t, _)| p == path && *t == changed).map(|(_, _, d)| d.clone()));
    let drawn = match found {
        Some(drawn) => drawn,
        None => {
            // Drawn with the cache let go: one drawing doesn't hold up the others.
            let drawn = std::fs::read(path).ok().and_then(|bytes| draw(&bytes, 2., path.parent()));
            if let Ok(mut cache) = DRAWN.lock() {
                cache.retain(|(p, _, _)| p != path);
                if cache.len() >= 32 {
                    cache.remove(0);
                }
                cache.push((path.to_path_buf(), changed, drawn.clone()));
            }
            drawn
        }
    };
    match drawn {
        Some((image, (width, height))) => {
            gpui::img(gpui::ImageSource::Render(image)).w(gpui::px(width)).h(gpui::px(height))
        }
        None => gpui::img(path.to_path_buf()),
    }
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
        let (image, size) = draw(svg, 2., None).expect("drawn");
        assert_eq!(size, (10., 5.));
        let pixels = image.as_bytes(0).unwrap();
        assert_eq!(pixels.len(), 20 * 10 * 4, "twice as many pixels each way");
        // Blue, green, red, alpha: the orange stays orange.
        assert_eq!(&pixels[..4], &[0x3a, 0xa3, 0xf2, 0xff]);
        assert!(draw(b"<svg", 2., None).is_none(), "not an SVG that draws");
    }

    #[test]
    fn svg_files_are_drawn_here_and_others_left_to_gpui() {
        let dir = crate::tools::test_dir("svg-source");
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("a.svg");
        std::fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#).unwrap();
        let drawn = |path: &std::path::Path| {
            image(path);
            DRAWN.lock().unwrap().iter().find(|(p, _, _)| p == path).map(|(_, _, d)| d.is_some())
        };
        assert_eq!(drawn(&svg), Some(true));
        assert_eq!(drawn(&dir.join("b.png")), None, "not an SVG: left to gpui");
        std::fs::write(&svg, "<svg").unwrap();
        assert_eq!(drawn(&svg), Some(false), "broken: kept as such, not read again each frame");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_huge_svg_is_drawn_smaller() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20000" height="100"/>"#;
        let (image, _) = draw(svg, 2., None).expect("drawn");
        assert!(image.size(0).width.0 <= 4096);
    }
}
