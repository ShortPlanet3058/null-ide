//! Files that aren't text. Images are shown as they are; anything else (a build output,
//! an archive, text in another encoding) gets a quiet line saying so. Neither is ever
//! edited or written back, so opening one can't damage it.

use std::path::Path;

/// The images shown in place of text. SVG stays text: it's code people edit.
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "avif", "qoi", "tga"];

/// Whether `path` is an image, by its name.
pub fn is_image(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    IMAGES.contains(&ext.as_str()) || ext == "svg"
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    Image { bytes: u64, size: Option<(u32, u32)> },
    NotText { bytes: u64 },
}

impl Preview {
    /// What the status bar says about it: "1280 × 720 · 245 KB".
    pub fn summary(&self) -> String {
        match self {
            Preview::Image { bytes, size: Some((w, h)) } => format!("{w} × {h} · {}", file_size(*bytes)),
            Preview::Image { bytes, size: None } | Preview::NotText { bytes } => file_size(*bytes),
        }
    }
}

/// How a file opens: `None` for text, the preview otherwise. `text` is what reading it as
/// text gave (see `encoding::read`), so the file is read once.
pub fn of(path: &Path, text: &Result<&str, std::io::ErrorKind>) -> Option<Preview> {
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if IMAGES.contains(&ext.as_str()) {
        let head = read_head(path);
        return Some(Preview::Image { bytes, size: image_size(&head) });
    }
    match text {
        // A NUL byte: UTF-8, maybe, but not text anyone types.
        Ok(text) if text.as_bytes().iter().take(8192).any(|&b| b == 0) => Some(Preview::NotText { bytes }),
        Ok(_) => None,
        Err(std::io::ErrorKind::InvalidData) => Some(Preview::NotText { bytes }),
        // Missing or unreadable: an empty editor, as before (a new file is written on save).
        Err(_) => None,
    }
}

fn read_head(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut head = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        file.take(64 * 1024).read_to_end(&mut head).ok();
    }
    head
}

pub fn file_size(bytes: u64) -> String {
    const KB: f64 = 1024.;
    match bytes as f64 {
        b if b < KB => format!("{bytes} bytes"),
        b if b < KB * KB => format!("{:.0} KB", b / KB),
        b if b < KB * KB * KB => format!("{:.1} MB", b / KB / KB),
        b => format!("{:.1} GB", b / KB / KB / KB),
    }
}

/// Width and height from an image's first bytes: PNG, GIF, BMP, JPEG and WebP.
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32);
    let le16 = |i: usize| Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32);
    let be32 = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let le32 = |i: usize| Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?));
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if b.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if b.starts_with(b"BM") {
        // Height is negative for images stored top-down.
        return Some((le32(18)?, (le32(22)? as i32).unsigned_abs()));
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        return match b.get(12..16)? {
            b"VP8 " => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            b"VP8L" => {
                let bits = le32(21)?;
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            b"VP8X" => {
                let le24 = |i: usize| Some(le32(i)? & 0xff_ffff);
                Some((le24(24)? + 1, le24(27)? + 1))
            }
            _ => None,
        };
    }
    if b.starts_with(&[0xff, 0xd8]) {
        // JPEG: walk the segments to the frame header, which holds the size.
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xff {
                return None;
            }
            let marker = b[i + 1];
            if marker == 0xff {
                i += 1;
                continue;
            }
            let frame = matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
            if frame {
                return Some((be16(i + 7)?, be16(i + 5)?));
            }
            i += 2 + be16(i + 2)? as usize;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_image_sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(1280u32.to_be_bytes());
        png.extend(720u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((1280, 720)));
        assert_eq!(image_size(b"GIF89a\x10\x00\x20\x00"), Some((16, 32)));
        // A JPEG with an APP0 segment before its frame header.
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x01, 0xe0, 0x02, 0x80, 0x03,
        ];
        assert_eq!(image_size(&jpeg), Some((640, 480)));
        assert_eq!(image_size(b"not an image"), None);
        assert_eq!(file_size(512), "512 bytes");
        assert_eq!(file_size(250_000), "244 KB");
        assert_eq!(file_size(5 * 1024 * 1024), "5.0 MB");
    }

    fn read(path: &Path) -> Option<Preview> {
        let read = crate::encoding::read(path);
        of(path, &read.as_ref().map(|(t, _)| t.as_str()).map_err(|e| e.kind()))
    }

    #[test]
    fn tells_text_from_the_rest() {
        let dir = std::env::temp_dir().join(format!("null-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let open = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            read(&path)
        };
        assert_eq!(open("a.rs", b"fn main() {}\n"), None);
        assert_eq!(open("a.svg", b"<svg/>"), None);
        assert_eq!(open("a.o", b"\xcf\xfa\xed\xfe\x07"), Some(Preview::NotText { bytes: 5 }));
        assert_eq!(open("nul.txt", b"a\0b"), Some(Preview::NotText { bytes: 3 }));
        assert!(matches!(open("a.png", b"\x89PNG"), Some(Preview::Image { size: None, .. })));
        assert_eq!(read(&dir.join("missing.rs")), None);
        // Text in an older encoding is text, not a binary.
        assert_eq!(open("latin.txt", b"caf\xe9\n"), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
