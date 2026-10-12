//! Colors written in code: `#f80`, `#ff8800cc`, `rgb(255 136 0)`, `hsl(30, 100%, 50%)`.
//! The editor shows a small square of each color just before it; a click on the square
//! picks another, written back the way the first was.

use gpui::{Hsla, Rgba};

/// The languages colors are written in: style sheets, markup, the scripts that set
/// styles, and the files that hold themes.
pub fn written_in(language: &str) -> bool {
    matches!(language, "CSS" | "HTML" | "JavaScript" | "TypeScript" | "TSX" | "JSON" | "YAML" | "TOML")
}

/// The colors in `text`: where each starts (a byte) and what it is.
pub fn colors_in(text: &str) -> Vec<(usize, Hsla)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Not inside a name: `my#fff`, `this.#abc` (a private field), `&#123;`.
        let starts_here =
            i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || matches!(bytes[i - 1], b'.' | b'&' | b'_'));
        let parsed = if !starts_here {
            None
        } else if bytes[i] == b'#' {
            hex(&text[i + 1..])
        } else if bytes[i].is_ascii_alphabetic() {
            function(&text[i..])
        } else {
            None
        };
        match parsed {
            Some((color, len)) => {
                found.push((i, color));
                i += len;
            }
            None => i += text[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    found
}

/// The color written at `byte` of `text` (where `colors_in` found one), and its length.
pub fn color_at(text: &str, byte: usize) -> Option<(Hsla, usize)> {
    let rest = text.get(byte..)?;
    if let Some(digits) = rest.strip_prefix('#') { hex(digits) } else { function(rest) }
}

/// `color` written the way `written` is: hex with as many digits (more when the alpha
/// needs them), or the same function, commas or spaces, with an alpha only when it
/// isn't opaque.
pub fn written_like(written: &str, color: Rgba) -> String {
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    let (r, g, b, a) = (byte(color.r), byte(color.g), byte(color.b), byte(color.a));
    let opaque = a == 255;
    if let Some(digits) = written.strip_prefix('#') {
        let upper = digits.chars().any(|c| c.is_ascii_uppercase());
        let short = digits.len() <= 4 && [r, g, b, a].iter().all(|v| v % 17 == 0);
        let text = match (short, opaque) {
            (true, true) => format!("#{:x}{:x}{:x}", r / 17, g / 17, b / 17),
            (true, false) => format!("#{:x}{:x}{:x}{:x}", r / 17, g / 17, b / 17, a / 17),
            (false, true) => format!("#{r:02x}{g:02x}{b:02x}"),
            (false, false) => format!("#{r:02x}{g:02x}{b:02x}{a:02x}"),
        };
        return if upper { text.to_uppercase() } else { text };
    }
    let name = written.split('(').next().unwrap_or("rgb").to_ascii_lowercase();
    let commas = written.contains(',');
    let alpha = {
        let a = (color.a.clamp(0., 1.) * 100.).round() / 100.;
        let text = format!("{a:.2}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    let (base, parts) = if name.starts_with("hsl") {
        let h: Hsla = color.into();
        let percent = |v: f32| format!("{}%", (v * 100.).round() as i32);
        ("hsl", [((h.h * 360.).round() as i32 % 360).to_string(), percent(h.s), percent(h.l)])
    } else {
        ("rgb", [r.to_string(), g.to_string(), b.to_string()])
    };
    match (commas, opaque) {
        (true, true) if name.ends_with('a') => format!("{base}a({}, {}, {}, 1)", parts[0], parts[1], parts[2]),
        (true, true) => format!("{base}({}, {}, {})", parts[0], parts[1], parts[2]),
        (true, false) => format!("{base}a({}, {}, {}, {alpha})", parts[0], parts[1], parts[2]),
        (false, true) => format!("{name}({} {} {})", parts[0], parts[1], parts[2]),
        (false, false) => format!("{name}({} {} {} / {alpha})", parts[0], parts[1], parts[2]),
    }
}

/// Whether `color`, drawn on `background`, would hardly show (near-black on a black
/// theme, a mostly see-through color): its swatch then gets a soft backdrop.
pub fn blends_into(color: Hsla, background: Hsla) -> bool {
    let (c, b): (Rgba, Rgba) = (color.into(), background.into());
    let over = |c: f32, b: f32| c * color.a + b * (1. - color.a);
    let light = |r: f32, g: f32, b: f32| 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let shown = light(over(c.r, b.r), over(c.g, b.g), over(c.b, b.b));
    (shown - light(b.r, b.g, b.b)).abs() < 0.12
}

/// `rgb`/`rgba`/`hsl`/`hsla` with its arguments: the color and the length taken
/// (counting the `#` for hex).
fn function(text: &str) -> Option<(Hsla, usize)> {
    // A short name right before `(`: looked for without reading the rest of the row.
    let name_len = text.bytes().take(5).take_while(u8::is_ascii_alphabetic).count();
    if !(3..=4).contains(&name_len) || text.as_bytes().get(name_len) != Some(&b'(') {
        return None;
    }
    let name = text[..name_len].to_ascii_lowercase();
    if !matches!(name.as_str(), "rgb" | "rgba" | "hsl" | "hsla") {
        return None;
    }
    // The arguments are short: a `)` far away isn't this one's.
    let close = text[name_len..].bytes().take(64).position(|b| b == b')')? + name_len;
    let inside = &text[name_len + 1..close];
    let parts: Vec<&str> = inside.split([',', ' ', '/']).map(str::trim).filter(|p| !p.is_empty()).collect();
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let number = |p: &str| p.trim_end_matches(['%']).trim_end_matches("deg").parse::<f32>().ok();
    let fraction = |p: &str| if p.ends_with('%') { number(p).map(|n| n / 100.) } else { number(p) };
    let alpha = match parts.get(3) {
        Some(a) => fraction(a)?.clamp(0., 1.),
        None => 1.,
    };
    let color = if name.starts_with("rgb") {
        let channel = |p: &str| {
            if p.ends_with('%') { number(p).map(|n| n / 100.) } else { number(p).map(|n| n / 255.) }
        };
        let rgba = Rgba {
            r: channel(parts[0])?.clamp(0., 1.),
            g: channel(parts[1])?.clamp(0., 1.),
            b: channel(parts[2])?.clamp(0., 1.),
            a: alpha,
        };
        rgba.into()
    } else {
        let h = number(parts[0])?.rem_euclid(360.) / 360.;
        gpui::hsla(h, fraction(parts[1])?.clamp(0., 1.), fraction(parts[2])?.clamp(0., 1.), alpha)
    };
    Some((color, close + 1))
}

/// The hex digits after a `#`: 3, 4, 6 or 8 of them, then not more of a name.
fn hex(text: &str) -> Option<(Hsla, usize)> {
    let digits = text.bytes().take_while(u8::is_ascii_hexdigit).count();
    if !matches!(digits, 3 | 4 | 6 | 8)
        || text[digits..].starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let value = |s: &str| u8::from_str_radix(s, 16).ok().map(|v| v as f32 / 255.);
    let short = |i: usize| value(&text[i..i + 1].repeat(2));
    let long = |i: usize| value(&text[i * 2..i * 2 + 2]);
    let (r, g, b, a) = match digits {
        3 => (short(0)?, short(1)?, short(2)?, 1.),
        4 => (short(0)?, short(1)?, short(2)?, short(3)?),
        6 => (long(0)?, long(1)?, long(2)?, 1.),
        _ => (long(0)?, long(1)?, long(2)?, long(3)?),
    };
    Some((Rgba { r, g, b, a }.into(), digits + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(c: Hsla) -> (u8, u8, u8, u8) {
        let c: Rgba = c.into();
        let byte = |v: f32| (v * 255.).round() as u8;
        (byte(c.r), byte(c.g), byte(c.b), byte(c.a))
    }

    #[test]
    fn a_picked_color_is_written_as_the_first_was() {
        let orange = Rgba { r: 1., g: 136. / 255., b: 0., a: 1. };
        let see_through = Rgba { a: 0.5, ..orange };
        let white = Rgba { r: 1., g: 1., b: 1., a: 1. };
        assert_eq!(written_like("#f80", orange), "#f80");
        assert_eq!(written_like("#f80", Rgba { g: 137. / 255., ..orange }), "#ff8900", "no short form for 137");
        assert_eq!(written_like("#f80", white), "#fff");
        assert_eq!(written_like("#FFF", orange), "#F80");
        assert_eq!(written_like("#ff8800", see_through), "#ff880080");
        assert_eq!(written_like("rgb(1, 2, 3)", orange), "rgb(255, 136, 0)");
        assert_eq!(written_like("rgb(1, 2, 3)", see_through), "rgba(255, 136, 0, 0.5)");
        assert_eq!(written_like("rgba(1, 2, 3, 0.2)", orange), "rgba(255, 136, 0, 1)");
        assert_eq!(written_like("rgb(1 2 3)", see_through), "rgb(255 136 0 / 0.5)");
        assert_eq!(written_like("hsl(0, 0%, 0%)", orange), "hsl(32, 100%, 50%)");
        assert_eq!(written_like("hsl(0 0% 0%)", see_through), "hsl(32 100% 50% / 0.5)");
        // And read back as the same color.
        let (back, len) = color_at("a: rgb(255 136 0 / 0.5);", 3).unwrap();
        assert_eq!((rgba(back), len), ((255, 136, 0, 128), 20));
        assert_eq!(color_at("x #ff8800", 2).map(|(_, len)| len), Some(7));
    }

    #[test]
    fn finds_colors_and_not_lookalikes() {
        let text = "a { color: #f80; background: #ff8800cc; border: rgb(255 136 0 / 50%); fill: hsl(30, 100%, 50%) }";
        let found: Vec<(usize, (u8, u8, u8, u8))> = colors_in(text).into_iter().map(|(b, c)| (b, rgba(c))).collect();
        assert_eq!(
            found,
            [(11, (255, 136, 0, 255)), (29, (255, 136, 0, 204)), (48, (255, 136, 0, 128)), (76, (255, 128, 0, 255)),]
        );
        // A private field, an id that isn't hex, a character reference, too many digits, a word.
        assert!(colors_in("this.#abc; #main; &#123; #1234567; #fade-in; srgb(1, 2, 3); rgb(a, b, c)").is_empty());
        assert_eq!(colors_in("'#FFF'").len(), 1);
        // Near-black, or mostly see-through, on black: hard to see.
        let black = gpui::black();
        let color = |text: &str| colors_in(text)[0].1;
        assert!(blends_into(color("#0b0b0c"), black));
        assert!(blends_into(color("rgba(245, 165, 36, 0.1)"), black));
        assert!(!blends_into(color("#f5a524"), black));
        assert!(blends_into(color("#fff"), gpui::white()));
    }
}
