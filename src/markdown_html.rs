//! A Markdown file as a page of its own (Export as HTML): what the preview shows, written
//! as HTML with a little CSS, to open in a browser, print to PDF or send.

use crate::markdown_view::{Align, Block, Callout, Inline};

/// The whole page for `source`, titled `title`.
pub fn page(source: &str, title: &str) -> String {
    let body = body(source);
    format!(
        "<!doctype html>\n{MARK}\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<main>\n{body}</main>\n</body>\n</html>\n",
        escape(title)
    )
}

/// `source` as HTML elements, without a page around them.
fn body(source: &str) -> String {
    let blocks: Vec<Block> = crate::markdown_view::parse_located(source).into_iter().map(|(_, b)| b).collect();
    let mut anchors = Anchors::default();
    let mut body = String::new();
    for block in &blocks {
        write_block(block, &mut body, &mut anchors);
    }
    body
}

/// `source` as the formatted text other apps paste (Mail, Notes, Pages, Docs).
pub fn fragment(source: &str) -> String {
    format!("<meta charset=\"utf-8\">{}", body(source))
}

/// Says a copy is Null's own Markdown, so pasting it back isn't read from its HTML.
#[cfg_attr(test, allow(dead_code))]
pub const OWN_COPY: &str = "dev.null-ide.markdown";

/// Puts `text` on the clipboard with `html` for apps that paste formatted text. Returns
/// false where that isn't done (the text alone should go then).
pub fn copy_rich(text: &str, html: &str) -> bool {
    // (A QA run keeps to a clipboard of its own: the text alone, there.)
    if crate::system_clipboard::kept_apart() {
        return false;
    }
    // Tests never touch the real clipboard.
    #[cfg(all(target_os = "macos", not(test)))]
    {
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
        use objc2_foundation::NSString;
        let board = NSPasteboard::generalPasteboard();
        board.clearContents();
        let text = NSString::from_str(text);
        let ok = board.setString_forType(&text, unsafe { NSPasteboardTypeString })
            && board.setString_forType(&NSString::from_str(html), unsafe { NSPasteboardTypeHTML });
        board.setString_forType(&text, &NSString::from_str(OWN_COPY));
        ok
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = (text, html);
        false
    }
}

/// A colour as CSS writes it: `#rrggbb`.
fn css(color: gpui::Hsla) -> String {
    let c: gpui::Rgba = color.into();
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

/// Code as a page shows it, in its colours: `text` cut into `(bytes, colour)` pieces (in
/// order; what's between them takes `plain`), on `background`, in `font`.
pub fn colored_code(
    text: &str,
    pieces: &[(std::ops::Range<usize>, gpui::Hsla)],
    plain: gpui::Hsla,
    background: gpui::Hsla,
    font: &str,
) -> String {
    let mut out = format!(
        "<pre style=\"font-family: '{}', Menlo, monospace; font-size: 13px; line-height: 1.5; color: {}; \
         background: {}; padding: 12px 16px; border-radius: 8px; white-space: pre;\">",
        escape(font),
        css(plain),
        css(background)
    );
    let mut at = 0;
    for (range, color) in pieces {
        let (start, end) = (range.start.max(at).min(text.len()), range.end.min(text.len()));
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        out.push_str(&escape(&text[at..start]));
        out.push_str(&format!("<span style=\"color: {}\">{}</span>", css(*color), escape(&text[start..end])));
        at = end;
    }
    out.push_str(&escape(&text[at..]));
    out.push_str("</pre>");
    out
}

/// The page's title: its first heading, if it has one.
pub fn title(source: &str) -> Option<String> {
    crate::markdown_view::parse_located(source).into_iter().find_map(|(_, block)| match block {
        Block::Heading(_, content) => Some(crate::markdown_view::plain_text(&content)),
        _ => None,
    })
}

/// Each heading's id, as GitHub makes them: its words as a slug, `-1`, `-2`… on repeats.
/// Taken from the headings as the page shows them, so they're in step with it.
#[derive(Default)]
struct Anchors(std::collections::HashMap<String, usize>);

impl Anchors {
    fn next(&mut self, content: &[Inline]) -> String {
        let base = crate::markdown_view::slug(&crate::markdown_view::plain_text(content));
        let n = self.0.entry(base.clone()).or_insert(0);
        let anchor = if *n == 0 { base } else { format!("{base}-{n}") };
        *n += 1;
        anchor
    }
}

/// Links and images that would run a script when clicked aren't kept: a page opened from
/// the disk runs them as the reader.
fn safe_url(url: &str, image: bool) -> &str {
    let scheme = url.trim_start().to_ascii_lowercase();
    let script = ["javascript:", "vbscript:"].iter().any(|s| scheme.starts_with(s));
    let data = scheme.starts_with("data:") && !(image && scheme.starts_with("data:image/"));
    if script || data { "#" } else { url }
}

/// Written at the top of every page Null exports: one found there is Null's to write over.
pub const MARK: &str = "<!-- Exported from Markdown by Null -->";

/// Calm, readable, in light and dark as the reader's system is.
const STYLE: &str = "
:root { color-scheme: light dark; --fg: #1d1e22; --muted: #6a6b72; --line: #e6e5e1; --code: #f4f4f2; --accent: #b4690e; }
@media (prefers-color-scheme: dark) { :root { --fg: #e8e6e3; --muted: #9a9890; --line: #2c2c33; --code: #16161a; --accent: #f5a524; } }
body { margin: 0; background: Canvas; color: var(--fg); font: 16px/1.6 -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; }
main { max-width: 46rem; margin: 0 auto; padding: 3rem 1.5rem; }
h1, h2 { border-bottom: 1px solid var(--line); padding-bottom: .3em; }
a { color: var(--accent); }
code, pre { font-family: ui-monospace, 'SF Mono', Menlo, monospace; font-size: .9em; background: var(--code); border-radius: 6px; }
code { padding: .1em .35em; }
pre { padding: 1em; overflow-x: auto; }
pre code { padding: 0; background: none; }
blockquote { margin: 0; padding-left: 1em; border-left: 3px solid var(--line); color: var(--muted); }
.callout { margin: 1em 0; padding-left: 1em; border-left: 3px solid var(--tint); }
.callout > .title { color: var(--tint); font-weight: 600; margin: 0; }
.note { --tint: #4a8fd8; } .tip { --tint: #3fa66b; } .important { --tint: var(--accent); } .warning { --tint: #c9a227; } .caution { --tint: #d9534f; }
table { border-collapse: collapse; }
th, td { border: 1px solid var(--line); padding: .35em .75em; }
img { max-width: 100%; }
hr { border: 0; border-top: 1px solid var(--line); }
li.task { list-style: none; } li.task input { margin: 0 .5em 0 -1.4em; }
";

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn write_block(block: &Block, out: &mut String, anchors: &mut Anchors) {
    match block {
        Block::Heading(level, content) => {
            let id = anchors.next(content);
            out.push_str(&format!("<h{level} id=\"{}\">{}</h{level}>\n", escape(&id), inlines(content)));
        }
        Block::Paragraph(content) => out.push_str(&format!("<p>{}</p>\n", inlines(content))),
        Block::Code { language, text, .. } => {
            let class =
                if language.is_empty() { String::new() } else { format!(" class=\"language-{}\"", escape(language)) };
            out.push_str(&format!("<pre><code{class}>{}</code></pre>\n", escape(text)));
        }
        Block::Quote(blocks) => {
            out.push_str("<blockquote>\n");
            blocks.iter().for_each(|b| write_block(b, out, anchors));
            out.push_str("</blockquote>\n");
        }
        Block::Callout(kind, blocks) => {
            let (class, name) = match kind {
                Callout::Note => ("note", "Note"),
                Callout::Tip => ("tip", "Tip"),
                Callout::Important => ("important", "Important"),
                Callout::Warning => ("warning", "Warning"),
                Callout::Caution => ("caution", "Caution"),
            };
            out.push_str(&format!("<div class=\"callout {class}\">\n<p class=\"title\">{name}</p>\n"));
            blocks.iter().for_each(|b| write_block(b, out, anchors));
            out.push_str("</div>\n");
        }
        Block::List { start, items } => {
            let (open, close) = match start {
                Some(1) => ("<ol>".to_string(), "</ol>"),
                Some(n) => (format!("<ol start=\"{n}\">"), "</ol>"),
                None => ("<ul>".to_string(), "</ul>"),
            };
            out.push_str(&open);
            out.push('\n');
            for item in items {
                match item.task {
                    Some(done) => {
                        let checked = if done { " checked" } else { "" };
                        out.push_str(&format!("<li class=\"task\"><input type=\"checkbox\" disabled{checked}>"));
                    }
                    None => out.push_str("<li>"),
                }
                // A one-paragraph item reads as its text, without a <p> around it.
                match item.blocks.as_slice() {
                    [Block::Paragraph(content)] => out.push_str(&inlines(content)),
                    blocks => blocks.iter().for_each(|b| write_block(b, out, anchors)),
                }
                out.push_str("</li>\n");
            }
            out.push_str(close);
            out.push('\n');
        }
        Block::Table { head, align, rows } => {
            let cell = |tag: &str, i: usize, content: &[Inline]| {
                let style = match align.get(i) {
                    Some(Align::Center) => " style=\"text-align:center\"",
                    Some(Align::Right) => " style=\"text-align:right\"",
                    _ => "",
                };
                format!("<{tag}{style}>{}</{tag}>", inlines(content))
            };
            out.push_str("<table>\n<thead><tr>");
            head.iter().enumerate().for_each(|(i, c)| out.push_str(&cell("th", i, c)));
            out.push_str("</tr></thead>\n<tbody>\n");
            for row in rows {
                out.push_str("<tr>");
                row.iter().enumerate().for_each(|(i, c)| out.push_str(&cell("td", i, c)));
                out.push_str("</tr>\n");
            }
            out.push_str("</tbody>\n</table>\n");
        }
        Block::Rule => out.push_str("<hr>\n"),
        Block::Note(label, content) => out.push_str(&format!("<p>[^{}]: {}</p>\n", escape(label), inlines(content))),
    }
}

fn inlines(content: &[Inline]) -> String {
    content
        .iter()
        .map(|inline| match inline {
            Inline::Text(text) => escape(text),
            Inline::Code(code) => format!("<code>{}</code>", escape(code)),
            Inline::Strong(inner) => format!("<strong>{}</strong>", inlines(inner)),
            Inline::Emphasis(inner) => format!("<em>{}</em>", inlines(inner)),
            Inline::Strike(inner) => format!("<del>{}</del>", inlines(inner)),
            Inline::Link { text, url } => format!("<a href=\"{}\">{}</a>", escape(safe_url(url, false)), inlines(text)),
            Inline::Image { alt, url, .. } => {
                format!("<img src=\"{}\" alt=\"{}\">", escape(safe_url(url, true)), escape(alt))
            }
            Inline::Break => "<br>\n".to_string(),
            Inline::NoteRef(label) => format!("[^{}]", escape(label)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a sample page to `$NULL_HTML_OUT`, to look at (run by hand).
    #[test]
    #[ignore]
    fn writes_a_sample_page() {
        let source = "# Field notes\n\nNull keeps what each save replaced[^1], so going back needs **no git**.\n\n> [!TIP]\n> Press ⌘K to find any command.\n\n## Keys\n\n| Key | Does |\n|:----|-----:|\n| ⌘P | Go to a file |\n| ⌘K | Commands |\n\n- [x] Spell check\n- [ ] Windows build\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```\n\n[^1]: Up to 50 versions a file, for 30 days.\n";
        std::fs::write(std::env::var("NULL_HTML_OUT").unwrap(), page(source, "Field notes")).unwrap();
    }

    #[test]
    fn a_document_becomes_a_page() {
        let source = "# Notes & more\n\nSome *words* with `code` and a [link](a.md)[^1].\n\n> [!TIP]\n> Use ⌘K.\n\n- [x] done\n- [ ] not yet\n\n| A | B |\n|:-:|--:|\n| 1 | <2> |\n\n```rust\nfn main() {}\n```\n\n[^1]: A note.\n";
        let html = page(source, "Notes");
        for expected in [
            "<title>Notes</title>",
            "<h1 id=\"notes--more\">Notes &amp; more</h1>",
            "<em>words</em>",
            "<code>code</code>",
            "<a href=\"a.md\">link</a>¹",
            "<div class=\"callout tip\">",
            "<li class=\"task\"><input type=\"checkbox\" disabled checked>done</li>",
            "<th style=\"text-align:center\">A</th>",
            "<td style=\"text-align:right\">&lt;2&gt;</td>",
            "<pre><code class=\"language-rust\">fn main() {}</code></pre>",
            "<li>A note.</li>",
        ] {
            assert!(html.contains(expected), "missing {expected}\n{html}");
        }
    }

    /// Ids follow the headings the page shows: one in a quote counts, a `#` line in a
    /// comment doesn't, and a repeat gets a number.
    #[test]
    fn a_fragment_is_the_page_without_the_page() {
        let html = fragment("# Notes\n\nSome **bold** and a [link](https://x.dev).\n");
        assert_eq!(
            html,
            "<meta charset=\"utf-8\"><h1 id=\"notes\">Notes</h1>\n<p>Some <strong>bold</strong> and a <a href=\"https://x.dev\">link</a>.</p>\n"
        );
        // And read back as the same Markdown.
        assert_eq!(
            crate::html_markdown::markdown(&html).as_deref(),
            Some("# Notes\n\nSome **bold** and a [link](https://x.dev).")
        );
    }

    #[test]
    fn heading_ids_stay_in_step() {
        let source = "# Setup\n\n<!--\n# not a heading\n-->\n\n> ## Aside\n\n## Usage\n\n## Usage\n";
        let html = page(source, "x");
        for expected in ["<h1 id=\"setup\">", "<h2 id=\"aside\">Aside</h2>", "<h2 id=\"usage\">", "<h2 id=\"usage-1\">"]
        {
            assert!(html.contains(expected), "missing {expected}\n{html}");
        }
        assert_eq!(title("<!--\n# no\n-->\n\nText\n\n# Yes\n").as_deref(), Some("Yes"));
    }

    #[test]
    fn script_links_are_not_kept() {
        let html = page(
            "[a](javascript:alert(1)) [b]( JavaScript:x) ![c](data:image/png;base64,AA) [d](data:text/html,x)",
            "x",
        );
        assert!(!html.to_lowercase().contains("javascript:"), "{html}");
        assert!(html.contains("src=\"data:image/png;base64,AA\""), "{html}");
        assert!(!html.contains("data:text/html"), "{html}");
    }

    #[test]
    fn code_is_copied_in_its_colours() {
        let red = gpui::Hsla::from(gpui::rgb(0xff0000));
        let white = gpui::Hsla::from(gpui::rgb(0xffffff));
        let black = gpui::Hsla::from(gpui::rgb(0x000000));
        let html = colored_code("let a = \"<b>\";", &[(0..3, red), (8..13, red)], white, black, "Geist Mono");
        assert!(html.starts_with("<pre style=\"font-family: 'Geist Mono', Menlo, monospace;"));
        assert!(html.contains("color: #ffffff; background: #000000"));
        assert!(html.contains("<span style=\"color: #ff0000\">let</span> a = <span style=\"color: #ff0000\">&quot;&lt;b&gt;&quot;</span>;</pre>"), "{html}");
        // A piece cut inside a character is left out (not a panic); one past the text, cut short.
        let html = colored_code("é", &[(0..1, red), (0..9, red)], white, black, "x");
        assert!(html.ends_with("><span style=\"color: #ff0000\">é</span></pre>"), "{html}");
    }
}
