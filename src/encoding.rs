//! How a file's bytes are text. Nearly everything is UTF-8; Null also reads, and writes
//! back as they were, UTF-8 with a byte-order mark (Windows tools, spreadsheets' CSVs),
//! UTF-16 with one, and Windows-1252 (old Western files, a superset of Latin-1).

use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Windows1252,
}

impl Encoding {
    /// Every encoding Null reads and writes.
    pub const ALL: [Encoding; 5] =
        [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Windows1252];

    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 with BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Windows1252 => "Windows-1252",
        }
    }
}

/// Windows-1252's characters for 0x80–0x9F; the rest of its bytes are Latin-1's. The five
/// it leaves undefined keep their control codes, so every byte reads and writes back.
const W1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{02C6}',
    '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}', '\u{0090}', '\u{2018}',
    '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}',
    '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

/// Reads a file as text. `InvalidData` when its bytes aren't text in any of the above.
pub fn read(path: &Path) -> std::io::Result<(String, Encoding)> {
    let bytes = std::fs::read(path)?;
    decode(bytes).ok_or_else(|| std::io::ErrorKind::InvalidData.into())
}

/// Reads a file as text only if that text, written back, is its bytes again: for changes
/// to a file that isn't open (a replace across the project, a rename's edits), which mustn't
/// touch what they don't change. (A few stray bytes among UTF-8 would come back as UTF-8.)
pub fn read_whole(path: &Path) -> std::io::Result<(String, Encoding)> {
    let bytes = std::fs::read(path)?;
    let (text, encoding) = decode(bytes.clone()).ok_or(std::io::ErrorKind::InvalidData)?;
    if encode(&text, encoding).ok().as_deref() != Some(bytes.as_slice()) {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok((text, encoding))
}

pub fn decode(bytes: Vec<u8>) -> Option<(String, Encoding)> {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8(rest.to_vec()).ok().map(|t| (t, Encoding::Utf8Bom));
    }
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| unit([c[0], c[1]])).collect();
        rest.len().is_multiple_of(2).then(|| String::from_utf16(&units).ok()).flatten()
    };
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return utf16(rest, u16::from_le_bytes).map(|t| (t, Encoding::Utf16Le));
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return utf16(rest, u16::from_be_bytes).map(|t| (t, Encoding::Utf16Be));
    }
    match String::from_utf8(bytes) {
        Ok(text) => Some((text, Encoding::Utf8)),
        Err(e) => {
            let bytes = e.into_bytes();
            if !looks_like_text(&bytes) {
                return None;
            }
            // UTF-8 with a few stray bytes (a Latin-1 "é" pasted in): read as UTF-8, the
            // strays as Windows-1252, so its own accents aren't garbled (and saved as UTF-8,
            // the file comes out whole). Mostly stray bytes: a Windows-1252 file.
            match mixed(&bytes) {
                Some(text) => Some((text, Encoding::Utf8)),
                None => Some((decode_1252(&bytes), Encoding::Windows1252)),
            }
        }
    }
}

/// `bytes` as UTF-8 with stray bytes in Windows-1252, when they're mostly UTF-8: more
/// characters written in UTF-8's several bytes than bytes that aren't UTF-8.
fn mixed(bytes: &[u8]) -> Option<String> {
    let (mut wide, mut stray) = (0, 0);
    for chunk in bytes.utf8_chunks() {
        wide += chunk.valid().chars().filter(|c| !c.is_ascii()).count();
        stray += chunk.invalid().len();
    }
    if wide <= stray {
        return None;
    }
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        text.push_str(&decode_1252(chunk.invalid()));
    }
    Some(text)
}

/// Decodes bytes known to be in `encoding` (the committed copy of a file, say).
pub fn decode_as(bytes: Vec<u8>, encoding: Encoding) -> Option<String> {
    match encoding {
        Encoding::Windows1252 => Some(decode_1252(&bytes)),
        // As the file itself was read: a few stray bytes among the UTF-8 included.
        Encoding::Utf8 => String::from_utf8(bytes).or_else(|e| mixed(e.as_bytes()).ok_or(())).ok(),
        _ => decode(bytes).map(|(text, _)| text),
    }
}

/// Bytes read as `encoding` and nothing else (its byte-order mark, if it has one, left
/// out): None when they aren't text in it.
pub fn decode_exactly(bytes: &[u8], encoding: Encoding) -> Option<String> {
    let utf16 = |bytes: &[u8], mark: &[u8], unit: fn([u8; 2]) -> u16| {
        let rest = bytes.strip_prefix(mark).unwrap_or(bytes);
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| unit([c[0], c[1]])).collect();
        rest.len().is_multiple_of(2).then(|| String::from_utf16(&units).ok()).flatten()
    };
    match encoding {
        Encoding::Utf8 | Encoding::Utf8Bom => {
            String::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes).to_vec()).ok()
        }
        // Nearly any bytes make UTF-16 or Windows-1252: only what reads as text is taken.
        Encoding::Utf16Le => utf16(bytes, b"\xFF\xFE", u16::from_le_bytes).filter(|t| reads_as_text(t)),
        Encoding::Utf16Be => utf16(bytes, b"\xFE\xFF", u16::from_be_bytes).filter(|t| reads_as_text(t)),
        Encoding::Windows1252 => looks_like_text(bytes).then(|| decode_1252(bytes)),
    }
}

/// Characters as text has them: no NUL, hardly any control characters but tabs and breaks.
fn reads_as_text(text: &str) -> bool {
    let mut count = 0;
    let mut controls = 0;
    for c in text.chars() {
        count += 1;
        if c == '\0' {
            return false;
        }
        if c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\u{0c}') {
            controls += 1;
        }
    }
    controls * 100 <= count
}

/// Text in a single-byte encoding: no NUL, and hardly any control characters other than
/// tabs and line breaks. Anything else (a build output, an archive) isn't text at all.
fn looks_like_text(bytes: &[u8]) -> bool {
    let controls =
        bytes.iter().filter(|&&b| (b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0c)) || b == 0x7f).count();
    !bytes.contains(&0) && controls * 100 <= bytes.len()
}

fn decode_1252(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if (0x80..0xA0).contains(&b) { W1252_HIGH[(b - 0x80) as usize] } else { b as char }).collect()
}

/// The bytes to write, or the first character the encoding can't hold.
pub fn encode(text: &str, encoding: Encoding) -> Result<Vec<u8>, char> {
    Ok(match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => [b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat(),
        Encoding::Utf16Le => [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect(),
        Encoding::Utf16Be => [0xFE, 0xFF].into_iter().chain(text.encode_utf16().flat_map(u16::to_be_bytes)).collect(),
        Encoding::Windows1252 => {
            let mut out = Vec::with_capacity(text.len());
            for c in text.chars() {
                let byte = match c as u32 {
                    n @ (0..0x80 | 0xA0..0x100) => n as u8,
                    _ => match W1252_HIGH.iter().position(|&h| h == c) {
                        Some(i) => 0x80 + i as u8,
                        None => return Err(c),
                    },
                };
                out.push(byte);
            }
            out
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_files_that_write_back_the_same_are_read_whole() {
        let dir = crate::tools::test_dir("read-whole");
        std::fs::create_dir_all(&dir).unwrap();
        let mixed = dir.join("mixed.txt");
        let mut bytes = "café, naïve, ".as_bytes().to_vec();
        bytes.extend([b'r', 0xE9, b's', b'u', b'm', b'\n']);
        std::fs::write(&mixed, &bytes).unwrap();
        assert!(read(&mixed).is_ok());
        assert!(read_whole(&mixed).is_err(), "its stray byte would be written as UTF-8");
        let latin = dir.join("latin.txt");
        std::fs::write(&latin, [b'r', 0xE9, b's', b'u', b'm', b'\n']).unwrap();
        assert_eq!(read_whole(&latin).unwrap(), ("résum\n".to_string(), Encoding::Windows1252));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn utf8_with_a_stray_byte_keeps_its_accents() {
        // "café, naïve, ", then a Latin-1 "é" (0xE9) pasted in.
        let mut bytes = "café, naïve, ".as_bytes().to_vec();
        bytes.extend([b'r', 0xE9, b's', b'u', b'm', b'\n']);
        assert_eq!(decode(bytes), Some(("café, naïve, résum\n".to_string(), Encoding::Utf8)));
        // A Windows-1252 file (its accents all single bytes) is still one.
        let latin = b"caf\xE9, na\xEFve, r\xE9sum\xE9\n".to_vec();
        assert_eq!(decode(latin), Some(("café, naïve, résumé\n".to_string(), Encoding::Windows1252)));
    }

    #[test]
    fn bytes_read_as_one_encoding_only() {
        assert_eq!(decode_exactly(b"\xC3\xA9", Encoding::Utf8).as_deref(), Some("é"));
        assert_eq!(decode_exactly(b"\xC3\xA9", Encoding::Windows1252).as_deref(), Some("Ã©"));
        assert_eq!(decode_exactly(b"\xFF\xFEh\0", Encoding::Utf16Le).as_deref(), Some("h"));
        assert_eq!(decode_exactly(b"\0h", Encoding::Utf16Be).as_deref(), Some("h"));
        assert_eq!(decode_exactly(b"\xE9", Encoding::Utf8), None);
        assert_eq!(decode_exactly(b"abc", Encoding::Utf16Le), None, "an odd number of bytes");
        // A program's bytes are no encoding's text.
        let binary = [0xCF, 0xFA, 0xED, 0xFE, 0x07, 0x00, 0x00, 0x01, 0x03, 0x00, 0x00, 0x00];
        assert_eq!(decode_exactly(&binary, Encoding::Windows1252), None);
        assert_eq!(decode_exactly(&binary, Encoding::Utf16Le), None);
    }

    #[test]
    fn reads_and_writes_back_every_encoding() {
        let text = "café – “quoted” €5\n";
        for encoding in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Windows1252]
        {
            let bytes = encode(text, encoding).unwrap();
            assert_eq!(decode(bytes.clone()), Some((text.to_string(), encoding)), "{encoding:?}");
            assert_eq!(encode(&decode(bytes.clone()).unwrap().0, encoding).unwrap(), bytes);
        }
        // Latin-1 bytes, as an old editor wrote them.
        assert_eq!(decode(b"caf\xe9\n".to_vec()), Some(("café\n".into(), Encoding::Windows1252)));
    }

    #[test]
    fn says_what_can_t_be_written() {
        assert_eq!(encode("ok 😀", Encoding::Windows1252), Err('😀'));
        assert!(encode("ok 😀", Encoding::Utf16Le).is_ok());
    }

    #[test]
    fn leaves_binaries_alone() {
        assert_eq!(decode(b"\xcf\xfa\xed\xfe\x07\x00\x00\x01".to_vec()), None);
        assert_eq!(decode(b"\x01\x02\x03\x04\xff\xfe\x05".to_vec()), None);
    }
}
