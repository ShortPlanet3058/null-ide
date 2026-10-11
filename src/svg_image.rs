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

/// A drawing made from text: drawn, not drawable, or still being drawn.
pub enum Made {
    Ready(Drawn),
    Failed,
    Waiting,
}

/// Making an SVG to draw (in the background).
pub type Job = Box<dyn FnOnce() -> Option<String> + Send>;

/// Drawings made from text (a formula, a diagram), the ones used last at the end; those
/// asked for and not drawn yet; and pictures let go, for their window to free.
#[derive(Default)]
struct MadeCache {
    /// Each drawing, and the frame it was last used in.
    done: Vec<(String, Option<Drawn>, u64)>,
    /// Counts the frames that drew from here.
    frame: u64,
    waiting: Vec<(String, Job)>,
    asked: std::collections::HashSet<String>,
    let_go: Vec<Arc<RenderImage>>,
}

static MADE: LazyLock<std::sync::Mutex<MadeCache>> = LazyLock::new(Default::default);

/// How many drawings made from text are kept.
const MADE_KEPT: usize = 256;

/// The drawing of the SVG `make` gives, remembered by `key` (what it's made from): made and
/// drawn once, in the background (see `take_waiting`), not as the preview draws.
pub fn made(key: &str, make: impl FnOnce() -> Option<String> + Send + 'static) -> Made {
    let Ok(mut cache) = MADE.lock() else { return Made::Failed };
    let frame = cache.frame;
    if let Some(at) = cache.done.iter().position(|(k, _, _)| k == key) {
        // Used: last to go.
        let mut entry = cache.done.remove(at);
        entry.2 = frame;
        let made = match &entry.1 {
            Some(drawn) => Made::Ready(drawn.clone()),
            None => Made::Failed,
        };
        cache.done.push(entry);
        return made;
    }
    if cache.asked.insert(key.to_string()) {
        cache.waiting.push((key.to_string(), Box::new(make)));
    }
    Made::Waiting
}

/// The drawings asked for, to make in the background, each then given to `finish`.
/// A new frame drew: the drawings used in it and the one before are kept, however many.
pub fn take_waiting() -> Vec<(String, Job)> {
    MADE.lock()
        .map(|mut cache| {
            cache.frame += 1;
            std::mem::take(&mut cache.waiting)
        })
        .unwrap_or_default()
}

/// Makes and draws `job` (sharp on a Retina screen); a panic in it is a drawing that
/// failed, not a crash.
pub fn make(job: Job) -> Option<Drawn> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job().and_then(|svg| draw(svg.as_bytes(), 2., None))))
        .ok()
        .flatten()
}

/// The drawing for `key`, made: kept, the one used longest ago let go past the limit.
pub fn finish(key: String, drawn: Option<Drawn>) {
    let Ok(mut cache) = MADE.lock() else { return };
    cache.asked.remove(&key);
    let frame = cache.frame;
    cache.done.push((key, drawn, frame));
    // Past the limit: the ones used longest ago, but none the screen shows now (they'd be
    // drawn again and again).
    while cache.done.len() > MADE_KEPT {
        let Some(old) = cache.done.iter().position(|(_, _, used)| *used + 1 < frame) else { break };
        if let (_, Some((image, _)), _) = cache.done.remove(old) {
            cache.let_go.push(image);
        }
    }
}

/// Pictures no longer kept, for the window to free on the GPU.
pub fn take_let_go() -> Vec<Arc<RenderImage>> {
    MADE.lock().map(|mut cache| std::mem::take(&mut cache.let_go)).unwrap_or_default()
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

    /// Pictures inside an SVG: a PNG written into it, a JPEG beside it.
    #[test]
    fn pictures_in_an_svg_are_drawn() {
        use base64::Engine as _;
        const RED_PNG: &str =
            "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAEElEQVR4nGP4z8AARwzEcQCukw/x0F8jngAAAABJRU5ErkJggg==";
        const RED_JPEG: &str = "/9j/4AAQSkZJRgABAQAASABIAAD/4QBMRXhpZgAATU0AKgAAAAgAAYdpAAQAAAABAAAAGgAAAAAAA6ABAAMAAAABAAEAAKACAAQAAAABAAAABKADAAQAAAABAAAABAAAAAD/7QA4UGhvdG9zaG9wIDMuMAA4QklNBAQAAAAAAAA4QklNBCUAAAAAABDUHYzZjwCyBOmACZjs+EJ+/8AAEQgABAAEAwEiAAIRAQMRAf/EAB8AAAEFAQEBAQEBAAAAAAAAAAABAgMEBQYHCAkKC//EALUQAAIBAwMCBAMFBQQEAAABfQECAwAEEQUSITFBBhNRYQcicRQygZGhCCNCscEVUtHwJDNicoIJChYXGBkaJSYnKCkqNDU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6g4SFhoeIiYqSk5SVlpeYmZqio6Slpqeoqaqys7S1tre4ubrCw8TFxsfIycrS09TV1tfY2drh4uPk5ebn6Onq8fLz9PX29/j5+v/EAB8BAAMBAQEBAQEBAQEAAAAAAAABAgMEBQYHCAkKC//EALURAAIBAgQEAwQHBQQEAAECdwABAgMRBAUhMQYSQVEHYXETIjKBCBRCkaGxwQkjM1LwFWJy0QoWJDThJfEXGBkaJicoKSo1Njc4OTpDREVGR0hJSlNUVVZXWFlaY2RlZmdoaWpzdHV2d3h5eoKDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uLj5OXm5+jp6vLz9PX29/j5+v/bAEMAAgICAgICAwICAwUDAwMFBgUFBQUGCAYGBgYGCAoICAgICAgKCgoKCgoKCgwMDAwMDA4ODg4ODw8PDw8PDw8PD//bAEMBAgICBAQEBwQEBxALCQsQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEP/dAAQAAf/aAAwDAQACEQMRAD8A+L6KKK/lM/38P//Z";
        let red = |pixels: &[u8]| pixels[2] > 200 && pixels[0] < 60 && pixels[1] < 60 && pixels[3] == 0xff;
        let inline = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4"><image width="4" height="4" xlink:href="data:image/png;base64,{RED_PNG}"/></svg>"#
        );
        let (image, _) = draw(inline.as_bytes(), 1., None).expect("drawn");
        assert!(red(&image.as_bytes(0).unwrap()[..4]), "the PNG");
        let dir = crate::tools::test_dir("svg-pictures");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("red.jpg"), base64::engine::general_purpose::STANDARD.decode(RED_JPEG).unwrap())
            .unwrap();
        let beside = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><image width="4" height="4" href="red.jpg"/></svg>"#;
        let (image, _) = draw(beside, 1., Some(&dir)).expect("drawn");
        let pixels = image.as_bytes(0).unwrap();
        assert!(red(&pixels[(2 * 4 + 2) * 4..][..4]), "the JPEG: {:?}", &pixels[..4]);
        std::fs::remove_dir_all(&dir).ok();
    }

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
