//! HTML as Markdown, for what's copied from a web page, Notes, Pages or Google Docs and
//! pasted in a Markdown file: headings, paragraphs, links, bold and italic, code, lists,
//! quotes, tables, images and rules. Made to read real copied HTML: tags left open, styles
//! standing for bold and italic, scripts and styles left out.

/// The formatted (HTML) version of what's on the clipboard, when the app it came from
/// gave one (a browser, Notes, Pages, Google Docs). Never for what Null copied: it gives
/// plain text only, which clears the rest.
pub fn clipboard_html() -> Option<String> {
    // Tests never read the real clipboard.
    #[cfg(all(target_os = "macos", not(test)))]
    {
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML};
        use objc2_foundation::NSString;
        let board = NSPasteboard::generalPasteboard();
        // Null's own Markdown, copied as rich text: the Markdown is the text already.
        if board.stringForType(&NSString::from_str(crate::markdown_html::OWN_COPY)).is_some() {
            return None;
        }
        let html = board.stringForType(unsafe { NSPasteboardTypeHTML })?;
        Some(html.to_string())
    }
    #[cfg(any(not(target_os = "macos"), test))]
    None
}

/// The Markdown for `html`, or None when there's nothing in it.
pub fn markdown(html: &str) -> Option<String> {
    let html = html.replace("\r\n", "\n");
    let nodes = parse(&html);
    let mut out = Writer::default();
    out.blocks(&nodes);
    out.flush();
    let text = out.blocks.join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

/// The text of `html` as a reader sees it, spaces run together: to tell whether it's the
/// same as the plain text copied with it.
pub fn plain_text(html: &str) -> String {
    fn collect(nodes: &[Node], out: &mut String) {
        for node in nodes {
            match node {
                Node::Text(text) => out.push_str(text),
                Node::Element { name, children, .. } => {
                    out.push(' ');
                    if !matches!(name.as_str(), "script" | "style" | "head" | "title") {
                        collect(children, out);
                    }
                    out.push(' ');
                }
            }
        }
    }
    let mut out = String::new();
    collect(&parse(html), &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug)]
enum Node {
    Text(String),
    Element { name: String, attributes: Vec<(String, String)>, children: Vec<Node> },
}

impl Node {
    fn attribute(&self, key: &str) -> Option<&str> {
        match self {
            Node::Element { attributes, .. } => attributes.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()),
            Node::Text(_) => None,
        }
    }
}

/// Elements with no content and no closing tag.
const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "wbr"];

/// What's in these is never text.
const SKIPPED: &[&str] = &["script", "style", "head", "title", "template", "noscript"];

/// An element still open: its name, attributes and what it holds so far.
type Open = (String, Vec<(String, String)>, Vec<Node>);

/// A tolerant tree: an end tag closes the element it names (and any left open inside
/// it), an end tag for nothing open is ignored, and a new `<p>` or `<li>` closes the one
/// before.
fn parse(html: &str) -> Vec<Node> {
    // The open elements, outermost first, each with what it holds so far.
    let mut stack: Vec<Open> = vec![(String::new(), Vec::new(), Vec::new())];
    let close_to = |stack: &mut Vec<Open>, depth: usize| {
        while stack.len() > depth {
            let (name, attributes, children) = stack.pop().expect("deeper than the root");
            if let Some(parent) = stack.last_mut() {
                parent.2.push(Node::Element { name, attributes, children });
            }
        }
    };
    let lower = html.to_ascii_lowercase();
    let mut rest = html;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            push_text(&mut stack.last_mut().expect("the root").2, rest);
            break;
        };
        push_text(&mut stack.last_mut().expect("the root").2, &rest[..open]);
        rest = &rest[open..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        let Some(end) = rest.find('>') else {
            push_text(&mut stack.last_mut().expect("the root").2, rest);
            break;
        };
        let tag = &rest[1..end];
        rest = &rest[end + 1..];
        if tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim().to_ascii_lowercase();
            if let Some(depth) = stack.iter().rposition(|(n, _, _)| *n == name) {
                close_to(&mut stack, depth);
            }
            continue;
        }
        let (name, attributes) = tag_parts(tag);
        if name.is_empty() {
            continue;
        }
        // Inside skipped elements, skip to their end tag.
        if SKIPPED.contains(&name.as_str()) {
            let end_tag = format!("</{name}");
            // Looked for in the page lowered once (the same bytes: only ASCII changes).
            let from = html.len() - rest.len();
            let found = lower[from..].find(&end_tag);
            rest = found.map_or("", |at| rest[at..].find('>').map_or("", |e| &rest[at + e + 1..]));
            continue;
        }
        // A new paragraph or item closes the one still open.
        if matches!(name.as_str(), "p" | "li" | "tr" | "td" | "th")
            && let Some(depth) = stack.iter().rposition(|(n, _, _)| *n == name)
            && !stack[depth + 1..].iter().any(|(n, _, _)| matches!(n.as_str(), "ul" | "ol" | "table"))
        {
            close_to(&mut stack, depth);
        }
        let self_closing = tag.trim_end().ends_with('/');
        if VOID.contains(&name.as_str()) || self_closing {
            stack.last_mut().expect("the root").2.push(Node::Element { name, attributes, children: Vec::new() });
        } else {
            stack.push((name, attributes, Vec::new()));
        }
    }
    close_to(&mut stack, 1);
    stack.pop().map(|(_, _, nodes)| nodes).unwrap_or_default()
}

fn push_text(nodes: &mut Vec<Node>, raw: &str) {
    if !raw.is_empty() {
        nodes.push(Node::Text(entities(raw)));
    }
}

/// A tag's name and attributes (`a href="x" title='y' hidden`), names in lower case.
fn tag_parts(tag: &str) -> (String, Vec<(String, String)>) {
    let tag = tag.trim_end_matches('/');
    let name_end = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
    let name = tag[..name_end].to_ascii_lowercase();
    let mut attributes = Vec::new();
    let mut chars = tag[name_end..].chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let mut key = String::new();
        while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '=') {
            key.push(c);
        }
        if key.is_empty() {
            break;
        }
        let mut value = String::new();
        if chars.next_if_eq(&'=').is_some() {
            match chars.peek().copied() {
                Some(quote @ ('"' | '\'')) => {
                    chars.next();
                    value.extend(chars.by_ref().take_while(|&c| c != quote));
                }
                _ => {
                    while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                        value.push(c);
                    }
                }
            }
        }
        attributes.push((key.to_ascii_lowercase(), entities(&value)));
    }
    (name, attributes)
}

/// `&amp;`, `&lt;`, `&#233;`, `&#xE9;`, `&nbsp;`… as the characters they stand for.
fn entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| {
            let name = &rest[1..e];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => ' ',
                "mdash" => '—',
                "ndash" => '–',
                "hellip" => '…',
                "rsquo" => '’',
                "lsquo" => '‘',
                "rdquo" => '”',
                "ldquo" => '“',
                _ => {
                    let number = name.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Builds the Markdown: blocks (paragraphs, headings, lists…) separated by blank lines,
/// each from the inline text that makes it.
#[derive(Default)]
struct Writer {
    blocks: Vec<String>,
    /// Inline text of the paragraph being made.
    line: String,
}

/// Whether `node` is a block, or holds one somewhere inside.
fn holds_blocks(node: &Node) -> bool {
    match node {
        Node::Element { name, children, .. } => is_block(name) || children.iter().any(holds_blocks),
        Node::Text(_) => false,
    }
}

/// Elements that make blocks of their own.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "nav"
            | "aside"
            | "figure"
            | "figcaption"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "blockquote"
            | "pre"
            | "hr"
            | "table"
            | "body"
            | "html"
            | "dl"
            | "dt"
            | "dd"
    )
}

impl Writer {
    /// The paragraph made so far becomes a block.
    fn flush(&mut self) {
        let text =
            self.line.split('\n').map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>();
        let text = text.join("  \n").trim_matches([' ', '\n']).to_string();
        if !text.is_empty() {
            self.blocks.push(text);
        }
        self.line.clear();
    }

    fn blocks(&mut self, nodes: &[Node]) {
        for node in nodes {
            match node {
                Node::Element { name, children, .. } if is_block(name) => {
                    self.flush();
                    self.block(node, name, children);
                }
                Node::Element { name, .. } if name == "br" => self.line.push('\n'),
                // An inline element around blocks (Google Docs wraps a whole document in a
                // <b>): its blocks, each on its own.
                Node::Element { children, .. } if children.iter().any(holds_blocks) => {
                    self.flush();
                    self.blocks(children);
                    self.flush();
                }
                node => {
                    let text = inline(node);
                    self.line.push_str(&text);
                }
            }
        }
    }

    fn block(&mut self, node: &Node, name: &str, children: &[Node]) {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name[1..].parse::<usize>().unwrap_or(1);
                let text = inline_all(children).split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    self.blocks.push(format!("{} {text}", "#".repeat(level)));
                }
            }
            "ul" | "ol" => {
                if let Some(list) = list(children, name == "ol", node.attribute("start")) {
                    self.blocks.push(list);
                }
            }
            "blockquote" => {
                let mut inner = Writer::default();
                inner.blocks(children);
                inner.flush();
                let quoted: Vec<String> = inner
                    .blocks
                    .join("\n\n")
                    .lines()
                    .map(|l| if l.is_empty() { ">".to_string() } else { format!("> {l}") })
                    .collect();
                if !quoted.is_empty() {
                    self.blocks.push(quoted.join("\n"));
                }
            }
            "pre" => {
                let code = raw_text(children);
                let class = node
                    .attribute("class")
                    .map(str::to_string)
                    .or_else(|| children.iter().find_map(|c| c.attribute("class").map(str::to_string)));
                let language = class
                    .as_deref()
                    .and_then(|c| c.split_whitespace().find_map(|w| w.strip_prefix("language-")))
                    .unwrap_or("");
                let fence = if code.contains("```") { "~~~" } else { "```" };
                self.blocks.push(format!("{fence}{language}\n{}\n{fence}", code.trim_end_matches('\n')));
            }
            "hr" => self.blocks.push("---".into()),
            "table" => {
                if let Some(table) = table(children) {
                    self.blocks.push(table);
                }
            }
            _ => {
                self.blocks(children);
                self.flush();
            }
        }
    }
}

/// A list's items as Markdown lines, nested lists indented under their item.
fn list(children: &[Node], numbered: bool, start: Option<&str>) -> Option<String> {
    let mut n = start.and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(1).min(999_999_999);
    let mut lines = Vec::new();
    // How far in the last item's text is: a list placed right in the list (not in an item,
    // as Google Docs writes them) goes under it.
    let mut pad = String::new();
    for child in children {
        let Node::Element { name, children: item, .. } = child else { continue };
        if matches!(name.as_str(), "ul" | "ol") {
            if let Some(nested) = list(item, name == "ol", child.attribute("start")) {
                lines.extend(nested.lines().map(|l| if l.is_empty() { String::new() } else { format!("{pad}{l}") }));
            }
            continue;
        }
        if name != "li" {
            continue;
        }
        let marker = if numbered { format!("{n}.") } else { "-".to_string() };
        n = n.saturating_add(1);
        pad = " ".repeat(marker.len() + 1);
        let mut inner = Writer::default();
        inner.blocks(item);
        inner.flush();
        let mut first = true;
        for block in &inner.blocks {
            for line in block.lines() {
                if first {
                    lines.push(format!("{marker} {line}"));
                    first = false;
                } else if line.is_empty() {
                    lines.push(String::new());
                } else {
                    lines.push(format!("{pad}{line}"));
                }
            }
        }
        if first {
            lines.push(marker);
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// A table's rows as a Markdown table, its first row the heading.
fn table(children: &[Node]) -> Option<String> {
    fn rows(nodes: &[Node], out: &mut Vec<Vec<String>>) {
        for node in nodes {
            let Node::Element { name, children, .. } = node else { continue };
            match name.as_str() {
                "tr" => out.push(
                    children
                        .iter()
                        .filter(|c| matches!(c, Node::Element { name, .. } if name == "td" || name == "th"))
                        .map(|c| match c {
                            Node::Element { children, .. } => inline_all(children)
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ")
                                .replace('|', "\\|"),
                            Node::Text(_) => String::new(),
                        })
                        .collect(),
                ),
                _ => rows(children, out),
            }
        }
    }
    let mut found = Vec::new();
    rows(children, &mut found);
    let width = found.iter().map(Vec::len).max().filter(|&w| w > 0)?;
    let line = |cells: &[String]| {
        let mut cells = cells.to_vec();
        cells.resize(width, String::new());
        format!("| {} |", cells.join(" | "))
    };
    let mut lines = vec![line(&found[0]), format!("|{}", "---|".repeat(width))];
    lines.extend(found[1..].iter().map(|r| line(r)));
    Some(lines.join("\n"))
}

/// Text as it's written, for a code block: line breaks and spaces kept.
fn raw_text(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|node| match node {
            Node::Text(text) => text.clone(),
            Node::Element { name, .. } if name == "br" => "\n".into(),
            Node::Element { children, .. } => raw_text(children),
        })
        .collect()
}

fn inline_all(nodes: &[Node]) -> String {
    nodes.iter().map(inline).collect()
}

/// `inner` between `mark`s, the spaces at its edges kept outside them (`** x **` isn't bold).
fn marked(inner: &str, mark: &str) -> String {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return inner.to_string();
    }
    let lead = &inner[..inner.len() - inner.trim_start().len()];
    let trail = &inner[inner.trim_end().len()..];
    format!("{lead}{mark}{trimmed}{mark}{trail}")
}

/// Text's Markdown marks taken literally.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '*' | '`' | '[' | ']' | '\\' | '<') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn inline(node: &Node) -> String {
    let Node::Element { name, children, .. } = node else {
        let Node::Text(text) = node else { unreachable!() };
        // Line breaks in the page's source are spaces: only <br> breaks a line.
        let mut collapsed = String::with_capacity(text.len());
        let mut space = false;
        for c in text.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                if !space {
                    collapsed.push(' ');
                }
                space = true;
            } else {
                collapsed.push(c);
                space = false;
            }
        }
        return escaped(&collapsed);
    };
    let inner = || inline_all(children);
    let style = node.attribute("style").unwrap_or("").to_ascii_lowercase().replace(' ', "");
    let bold_style = style.contains("font-weight:700")
        || style.contains("font-weight:bold")
        || style.contains("font-weight:600")
        || style.contains("font-weight:800");
    let italic_style = style.contains("font-style:italic");
    match name.as_str() {
        // Google Docs wraps everything in a <b> that isn't bold.
        "b" | "strong" if style.contains("font-weight:normal") || style.contains("font-weight:400") => inner(),
        "b" | "strong" => marked(&inner(), "**"),
        "i" | "em" => marked(&inner(), "*"),
        "del" | "s" | "strike" => marked(&inner(), "~~"),
        "code" | "kbd" | "samp" => {
            let code = raw_text(children).replace('\n', " ");
            if code.is_empty() {
                String::new()
            } else {
                // One more tick than the longest run inside, spaced when it starts or ends with one.
                let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                let ticks = "`".repeat(longest + 1);
                let pad = if code.starts_with('`') || code.ends_with('`') { " " } else { "" };
                format!("{ticks}{pad}{code}{pad}{ticks}")
            }
        }
        "a" => {
            let text = inner();
            match node.attribute("href").map(str::trim) {
                Some(href) if !href.is_empty() && !href.starts_with('#') && !href.starts_with("javascript:") => {
                    if text.trim().is_empty() { format!("<{href}>") } else { format!("[{}]({href})", text.trim()) }
                }
                _ => text,
            }
        }
        "img" => match node.attribute("src") {
            Some(src) if !src.is_empty() && !src.starts_with("data:") => {
                format!("![{}]({src})", node.attribute("alt").unwrap_or(""))
            }
            _ => String::new(),
        },
        "br" => "\n".into(),
        _ if bold_style && italic_style => marked(&marked(&inner(), "*"), "**"),
        _ if bold_style => marked(&inner(), "**"),
        _ if italic_style => marked(&inner(), "*"),
        _ => inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_web_page_becomes_markdown() {
        let html = r#"<meta charset="utf-8"><h2 id="x">Getting <em>started</em></h2>
            <p>Read the <a href="https://example.com/docs">docs</a> &amp; run <code>cargo build</code>.<br>Then <b>go</b>!</p>
            <ul><li>One<li>Two<ul><li>Nested</li></ul></li></ul>
            <ol start="3"><li>Third</li></ol>
            <blockquote><p>Quoted</p><p>Twice</p></blockquote>
            <pre><code class="language-rust">fn main() {
    println!("&lt;hi&gt;");
}</code></pre>
            <table><tr><th>Key</th><th>Does</th></tr><tr><td>⌘P</td><td>a | file</td></tr></table>
            <hr><p><img src="a.png" alt="A"> 5 * 3</p><script>alert(1)</script><style>p{}</style>"#;
        let md = markdown(html).unwrap();
        let expected = [
            "## Getting *started*",
            "Read the [docs](https://example.com/docs) & run `cargo build`.  \nThen **go**!",
            "- One\n- Two\n  - Nested",
            "3. Third",
            "> Quoted\n>\n> Twice",
            "```rust\nfn main() {\n    println!(\"<hi>\");\n}\n```",
            "| Key | Does |\n|---|---|\n| ⌘P | a \\| file |",
            "---",
            "![A](a.png) 5 \\* 3",
        ]
        .join("\n\n");
        assert_eq!(md, expected);
        assert!(!md.contains("alert"));
    }

    #[test]
    fn documents_bold_by_style() {
        // Google Docs: a <b> around everything that isn't bold, spans for bold and italic.
        let html = r#"<b style="font-weight:normal;" id="docs-internal-guid-1"><p><span style="font-weight:700">Bold</span> and <span style="font-style:italic">italic</span> text</p></b>"#;
        assert_eq!(markdown(html).as_deref(), Some("**Bold** and *italic* text"));
        assert_eq!(markdown("<p><b> spaced </b>out</p>").as_deref(), Some("**spaced** out"));
        assert_eq!(markdown("<p></p>  "), None);
    }

    #[test]
    fn what_documents_and_pages_really_send() {
        // Google Docs: one <b> around everything, lists placed right in their lists.
        let docs = r#"<b style="font-weight:normal"><h1>T</h1><p>A</p><ul><li>x</li><ul><li>under x</li></ul><li>y</li></ul></b>"#;
        assert_eq!(markdown(docs).as_deref(), Some("# T\n\nA\n\n- x\n  - under x\n- y"));
        // The source's line breaks are spaces; code with ticks; a tag written as text.
        assert_eq!(markdown("<p>Some long\ntext</p>").as_deref(), Some("Some long text"));
        assert_eq!(
            markdown("<p>Use <code>`x`</code> or <code>a``b</code></p>").as_deref(),
            Some("Use `` `x` `` or ```a``b```")
        );
        assert_eq!(markdown("<p>the &lt;br&gt; tag</p>").as_deref(), Some("the \\<br> tag"));
        assert_eq!(markdown("<pre>a\r\nb</pre>").as_deref(), Some("```\na\nb\n```"));
        assert_eq!(markdown(r#"<ol start="18446744073709551615"><li>a<li>b</ol>"#).as_deref(), Some("1. a\n2. b"));
    }

    #[test]
    fn the_plain_text_is_what_a_reader_sees() {
        assert_eq!(plain_text("<p>Hello <b>you</b></p><p>there&nbsp;!</p><style>x</style>"), "Hello you there !");
        assert_eq!(entities("&#233;t&#xE9; &unknown; & co"), "été &unknown; & co");
    }
}
