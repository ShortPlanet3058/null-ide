//! Math and diagrams in Markdown's preview: a formula (`$$ … $$`, a ```math block) drawn
//! as TeX lays it out (RaTeX, KaTeX's fonts inside), a ```mermaid block drawn as its
//! diagram (in the theme's colours), and a formula in a line of text (`$x^2$`) written
//! with the characters it reads as (x²), the line's text being text.

/// `latex` as an SVG drawing, in `color`, `size` points to an em; None when it doesn't
/// read as math.
pub fn formula_svg(latex: &str, color: gpui::Hsla, size: f32) -> Option<String> {
    let nodes = ratex_parser::parse(latex.trim()).ok()?;
    let rgba = gpui::Rgba::from(color);
    let options = ratex_layout::LayoutOptions {
        color: ratex_types::color::Color::new(rgba.r, rgba.g, rgba.b, rgba.a),
        ..Default::default()
    };
    // (A layout that panics on something it doesn't know isn't a crash: just not drawn.)
    let list =
        std::panic::catch_unwind(|| ratex_layout::to_display_list(&ratex_layout::layout(&nodes, &options))).ok()?;
    let svg_options =
        ratex_svg::SvgOptions { font_size: size as f64, padding: 2., embed_glyphs: true, ..Default::default() };
    Some(ratex_svg::render_to_svg(&list, &svg_options))
}

/// A Mermaid diagram as an SVG drawing, in the theme's colours on no background; None when
/// it doesn't read as one.
pub fn diagram_svg(source: &str, theme: &crate::theme::Theme, font: &str) -> Option<String> {
    let dark = theme.background.l < 0.5;
    let mut colours = if dark { mermaid_rs_renderer::Theme::dark() } else { mermaid_rs_renderer::Theme::modern() };
    let hex = |c: gpui::Hsla| {
        let c = gpui::Rgba::from(c);
        format!("#{:02x}{:02x}{:02x}", (c.r * 255.) as u8, (c.g * 255.) as u8, (c.b * 255.) as u8)
    };
    colours.font_family = format!("'{font}', sans-serif");
    colours.font_size = 14.;
    colours.background = "none".into();
    colours.primary_color = hex(theme.raised);
    colours.primary_text_color = hex(theme.foreground);
    colours.primary_border_color = hex(theme.line_strong);
    colours.text_color = hex(theme.foreground);
    colours.line_color = hex(theme.muted);
    colours.edge_label_background = hex(theme.background);
    colours.cluster_background = hex(theme.surface);
    colours.cluster_border = hex(theme.hairline);
    let options = mermaid_rs_renderer::RenderOptions { theme: colours, ..Default::default() };
    let source = source.to_string();
    std::panic::catch_unwind(move || mermaid_rs_renderer::render_with_options(&source, options).ok()).ok().flatten()
}

/// A formula written in a line of text, as the characters it reads as: `\alpha^2 \leq
/// \frac{1}{n}` is "α² ≤ 1⁄n". What has no character of its own stays as written, its
/// braces gone (`x^{ab}` is x^(ab)).
pub fn as_text(latex: &str) -> String {
    as_text_within(latex, 0)
}

/// `as_text`, `depth` groups in (very deep ones are left as they're written).
fn as_text_within(latex: &str, depth: usize) -> String {
    if depth > 32 {
        return latex.to_string();
    }
    let as_text = |inner: &str| as_text_within(inner, depth + 1);
    let chars: Vec<char> = latex.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                let name: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
                if name.is_empty() {
                    // `\,` `\;` `\!` `\ `: a space, or none; `\{`: the brace itself.
                    match chars.get(i + 1) {
                        Some(',' | ';' | ':' | ' ') => out.push(' '),
                        Some('!') | None => {}
                        Some(&other) => out.push(other),
                    }
                    i += 2;
                    continue;
                }
                i += 1 + name.len();
                match name.as_str() {
                    "frac" | "dfrac" | "tfrac" => {
                        let (top, after) = group(&chars, i);
                        let (bottom, after) = group(&chars, after);
                        i = after;
                        let (top, bottom) = (as_text(&top), as_text(&bottom));
                        let wrap = |s: String| if s.chars().count() > 1 { format!("({s})") } else { s };
                        out.push_str(&format!("{}⁄{}", wrap(top), wrap(bottom)));
                    }
                    "sqrt" => {
                        let (inside, after) = group(&chars, i);
                        i = after;
                        let inside = as_text(&inside);
                        if inside.chars().count() > 1 {
                            out.push_str(&format!("√({inside})"));
                        } else {
                            out.push('√');
                            out.push_str(&inside);
                        }
                    }
                    "text" | "mathrm" | "mathbf" | "mathit" | "mathsf" | "mathtt" | "operatorname" | "textbf" => {
                        let (inside, after) = group(&chars, i);
                        i = after;
                        out.push_str(&inside);
                    }
                    "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "displaystyle" | "limits" => {}
                    name => match symbol(name) {
                        Some(symbol) => out.push_str(symbol),
                        None => out.push_str(name),
                    },
                }
            }
            '^' | '_' => {
                let (inside, after) = group(&chars, i + 1);
                i = after;
                let inside = as_text(&inside);
                let up = c == '^';
                let small: Option<String> = inside.chars().map(|ch| scripted(ch, up)).collect();
                match small {
                    Some(small) => out.push_str(&small),
                    None => out.push_str(&format!("{c}({inside})")),
                }
            }
            '{' | '}' => i += 1,
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// The group at `i` (`{…}`, or the next character or command): its text and where it ends.
fn group(chars: &[char], mut i: usize) -> (String, usize) {
    while chars.get(i) == Some(&' ') {
        i += 1;
    }
    match chars.get(i) {
        Some('{') => {
            let mut depth = 0;
            let start = i + 1;
            while i < chars.len() {
                match chars[i] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return (chars[start..i].iter().collect(), i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            (chars[start..].iter().collect(), chars.len())
        }
        Some('\\') => {
            let name: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
            let end = i + 1 + name.len().max(1);
            (chars[i..end.min(chars.len())].iter().collect(), end.min(chars.len()))
        }
        Some(&c) => (c.to_string(), i + 1),
        None => (String::new(), i),
    }
}

/// `c` raised (`up`) or lowered, when there's a character for that.
fn scripted(c: char, up: bool) -> Option<char> {
    const DIGITS_UP: &str = "⁰¹²³⁴⁵⁶⁷⁸⁹";
    const DIGITS_DOWN: &str = "₀₁₂₃₄₅₆₇₈₉";
    if let Some(d) = c.to_digit(10) {
        return (if up { DIGITS_UP } else { DIGITS_DOWN }).chars().nth(d as usize);
    }
    let pairs: &[(char, char)] = if up {
        &[
            ('+', '⁺'),
            ('-', '⁻'),
            ('=', '⁼'),
            ('(', '⁽'),
            (')', '⁾'),
            ('n', 'ⁿ'),
            ('i', 'ⁱ'),
            ('a', 'ᵃ'),
            ('b', 'ᵇ'),
            ('c', 'ᶜ'),
            ('d', 'ᵈ'),
            ('e', 'ᵉ'),
            ('k', 'ᵏ'),
            ('m', 'ᵐ'),
            ('o', 'ᵒ'),
            ('p', 'ᵖ'),
            ('t', 'ᵗ'),
            ('x', 'ˣ'),
            ('y', 'ʸ'),
            ('T', 'ᵀ'),
            ('*', '*'),
            ('′', '′'),
        ]
    } else {
        &[
            ('+', '₊'),
            ('-', '₋'),
            ('=', '₌'),
            ('(', '₍'),
            (')', '₎'),
            ('a', 'ₐ'),
            ('e', 'ₑ'),
            ('i', 'ᵢ'),
            ('j', 'ⱼ'),
            ('k', 'ₖ'),
            ('m', 'ₘ'),
            ('n', 'ₙ'),
            ('o', 'ₒ'),
            ('p', 'ₚ'),
            ('r', 'ᵣ'),
            ('s', 'ₛ'),
            ('t', 'ₜ'),
            ('x', 'ₓ'),
        ]
    };
    pairs.iter().find(|(from, _)| *from == c).map(|(_, to)| *to)
}

/// The character a command names (`\alpha` α, `\leq` ≤).
fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" | "vartheta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" => "ρ",
        "sigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "times" => "×",
        "cdot" => "·",
        "pm" => "±",
        "mp" => "∓",
        "div" => "÷",
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "neq" | "ne" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "sim" => "∼",
        "propto" => "∝",
        "infty" => "∞",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "oint" => "∮",
        "partial" => "∂",
        "nabla" => "∇",
        "to" | "rightarrow" => "→",
        "leftarrow" => "←",
        "Rightarrow" | "implies" => "⇒",
        "Leftarrow" => "⇐",
        "leftrightarrow" => "↔",
        "Leftrightarrow" | "iff" => "⇔",
        "mapsto" => "↦",
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",
        "subset" => "⊂",
        "subseteq" => "⊆",
        "supset" => "⊃",
        "supseteq" => "⊇",
        "cup" => "∪",
        "cap" => "∩",
        "emptyset" | "varnothing" => "∅",
        "forall" => "∀",
        "exists" => "∃",
        "neg" => "¬",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "oplus" => "⊕",
        "otimes" => "⊗",
        "circ" => "∘",
        "ldots" | "dots" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "prime" => "′",
        "degree" => "°",
        "mid" => "∣",
        "parallel" => "∥",
        "perp" => "⊥",
        "angle" => "∠",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "aleph" => "ℵ",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "quad" | "qquad" => " ",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas_in_a_line_as_text() {
        assert_eq!(as_text(r"\alpha^2 \leq \frac{1}{n}"), "α² ≤ 1⁄n");
        assert_eq!(as_text(r"x_i + y_{10}"), "xᵢ + y₁₀");
        assert_eq!(as_text(r"e^{i\pi} = -1"), "e^(iπ) = -1");
        assert_eq!(as_text(r"\sqrt{x^2+1}"), "√(x²+1)");
        assert_eq!(as_text(r"\text{rate} \times 2"), "rate × 2");
        assert_eq!(as_text(r"\frac{a+b}{2}"), "(a+b)⁄2");
        assert_eq!(as_text(r"\{1, 2\}"), "{1, 2}");
    }

    #[test]
    fn formulas_and_diagrams_are_drawn() {
        let white = gpui::Hsla { h: 0., s: 0., l: 1., a: 1. };
        let svg = formula_svg(r"\frac{a}{b} + \sqrt{x}", white, 18.).unwrap();
        assert!(svg.contains("<svg") && svg.contains("<path"), "glyphs as outlines: {}", &svg[..svg.len().min(200)]);
        assert!(crate::svg_image::draw(svg.as_bytes(), 2., None).is_some(), "it draws");
        assert!(formula_svg(r"\frac{a}{", white, 18.).is_none(), "not math");
        let theme = crate::theme::Theme::oled();
        let diagram = diagram_svg("flowchart LR\n  A[Write] --> B[Read]\n", &theme, "Helvetica").unwrap();
        assert!(diagram.contains("Write"), "{}", &diagram[..diagram.len().min(200)]);
        assert!(crate::svg_image::draw(diagram.as_bytes(), 2., None).is_some(), "it draws");
        assert!(diagram_svg("not a diagram at all", &theme, "Helvetica").is_none());
    }
}
