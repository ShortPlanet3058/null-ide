//! Emmet abbreviations in HTML: `ul>li.item*3` then ⇥ writes the tags out, the places to
//! fill in visited with ⇥. Elements (`div`), classes (`.card`, alone a `div`), ids (`#top`),
//! attributes (`[type=email]`), text (`{Hello}`), children (`>`), siblings (`+`), climbing
//! back up (`^`), repeats (`*3`, with `$` numbering them). A plain word expands only when
//! it's an HTML element, so ordinary words stay words.

/// The abbreviation ending at the end of `before` (a line up to the caret), if there is one
/// to expand: where it starts (a byte) and the snippet it becomes, its lines after the first
/// indented by `indent`, each level deeper by `unit`.
pub fn expand_at_end(before: &str, indent: &str, unit: &str) -> Option<(usize, String)> {
    let start = abbreviation_start(before)?;
    let abbreviation = &before[start..];
    let nodes = parse(abbreviation)?;
    // A lone word: only an element's name (not "hello" in a sentence).
    let operators = abbreviation.contains(['.', '#', '>', '+', '*', '^', '[', '{']);
    if !operators && !ELEMENTS.contains(&abbreviation) {
        return None;
    }
    let mut out = Writer { text: String::new(), stop: 0, unit: unit.to_string() };
    out.nodes(&nodes, 0, indent);
    out.text.push_str("$0");
    Some((start, out.text))
}

/// Where the abbreviation before the caret starts: after a space or a tag's `>`, with
/// spaces allowed inside `{text}` and `[attributes]` only. None inside a tag being written
/// (`<a href="x`), after nothing, or after `<`.
fn abbreviation_start(before: &str) -> Option<usize> {
    let in_tag = before.rfind('<').is_some_and(|open| before.rfind('>').is_none_or(|close| close < open));
    if in_tag || before.is_empty() {
        return None;
    }
    let mut depth = 0i32;
    let mut start = before.len();
    for (i, c) in before.char_indices().rev() {
        match c {
            '}' | ']' => depth += 1,
            '{' | '[' => depth -= 1,
            _ if depth > 0 => {}
            c if c.is_whitespace() => break,
            // The end of a tag before it: `<p>` then `span`.
            '>' => {
                let tag = before[..i].rfind('<').map(|open| &before[open..i]);
                if tag.is_some_and(|t| !t.contains('>') && t.contains(|c: char| c.is_alphabetic())) {
                    break;
                }
            }
            _ => {}
        }
        if depth < 0 {
            return None;
        }
        start = i;
    }
    let abbreviation = &before[start..];
    let first = abbreviation.chars().next()?;
    (depth == 0 && (first.is_alphabetic() || matches!(first, '.' | '#' | '['))).then_some(start)
}

#[derive(Debug, Default, PartialEq)]
struct Node {
    tag: String,
    id: Option<String>,
    classes: Vec<String>,
    attributes: Vec<(String, Option<String>)>,
    text: Option<String>,
    children: Vec<Node>,
}

/// The tree an abbreviation stands for; None when it isn't one.
fn parse(abbreviation: &str) -> Option<Vec<Node>> {
    let chars: Vec<char> = abbreviation.chars().collect();
    let mut at = 0;
    // The open parents, outermost first: each holds the nodes being added to it.
    let mut levels: Vec<Vec<Node>> = vec![Vec::new()];
    loop {
        let (element, count) = element(&chars, &mut at)?;
        let parent = levels.last_mut()?;
        for n in 1..=count {
            parent.push(numbered(&element, n, count));
        }
        match chars.get(at) {
            None => break,
            Some('+') => {}
            Some('>') => levels.push(Vec::new()),
            Some('^') => {
                let mut ups = 0;
                while chars.get(at) == Some(&'^') {
                    ups += 1;
                    at += 1;
                }
                for _ in 0..ups {
                    close(&mut levels)?;
                }
                continue;
            }
            Some(_) => return None,
        }
        at += 1;
    }
    while levels.len() > 1 {
        close(&mut levels)?;
    }
    levels.pop()
}

/// The innermost open level goes inside the last node of the one around it (each copy of
/// a repeated one).
fn close(levels: &mut Vec<Vec<Node>>) -> Option<()> {
    if levels.len() < 2 {
        // Climbing past the top stays at the top.
        return Some(());
    }
    let children = levels.pop()?;
    let parent = levels.last_mut()?;
    let last = parent.last_mut()?;
    last.children = children;
    Some(())
}

/// One element and how many times it repeats, read from `at`.
fn element(chars: &[char], at: &mut usize) -> Option<(Node, usize)> {
    let name = |at: &mut usize| {
        let start = *at;
        while chars.get(*at).is_some_and(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '$' | '@')) {
            *at += 1;
        }
        chars[start..*at].iter().collect::<String>()
    };
    let mut node = Node { tag: name(at), ..Default::default() };
    let mut count = 1;
    loop {
        match chars.get(*at) {
            Some('.') => {
                *at += 1;
                node.classes.push(name(at));
            }
            Some('#') => {
                *at += 1;
                node.id = Some(name(at));
            }
            Some('[') => {
                *at += 1;
                let close = chars[*at..].iter().position(|&c| c == ']')? + *at;
                let inside: String = chars[*at..close].iter().collect();
                node.attributes.extend(inside.split_whitespace().map(|pair| match pair.split_once('=') {
                    Some((key, value)) => (key.to_string(), Some(value.trim_matches(['"', '\'']).to_string())),
                    None => (pair.to_string(), None),
                }));
                *at = close + 1;
            }
            Some('{') => {
                *at += 1;
                let close = chars[*at..].iter().position(|&c| c == '}')? + *at;
                node.text = Some(chars[*at..close].iter().collect());
                *at = close + 1;
            }
            Some('*') => {
                *at += 1;
                let digits = name(at);
                count = digits.parse().ok().filter(|n| (1..=100).contains(n))?;
            }
            _ => break,
        }
    }
    if node.classes.iter().any(String::is_empty) || node.id.as_deref() == Some("") {
        return None;
    }
    if node.tag.is_empty() {
        if node.classes.is_empty() && node.id.is_none() && node.attributes.is_empty() {
            return None;
        }
        node.tag = "div".into();
    }
    Some((node, count))
}

/// Copy `n` of `count` of a node: `$` (or `$$`, zero-padded) becomes its number.
fn numbered(node: &Node, n: usize, count: usize) -> Node {
    let number = |text: &str| -> String {
        if count == 1 && !text.contains('$') {
            return text.to_string();
        }
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '$' {
                let mut width = 1;
                while chars.next_if_eq(&'$').is_some() {
                    width += 1;
                }
                out.push_str(&format!("{n:0width$}"));
            } else {
                out.push(c);
            }
        }
        out
    };
    Node {
        tag: number(&node.tag),
        id: node.id.as_deref().map(number),
        classes: node.classes.iter().map(|c| number(c)).collect(),
        attributes: node.attributes.iter().map(|(k, v)| (number(k), v.as_deref().map(number))).collect(),
        text: node.text.as_deref().map(number),
        children: Vec::new(),
    }
}

/// HTML elements: a lone word that's one of these expands.
const ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "article",
    "aside",
    "audio",
    "b",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "code",
    "dd",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "em",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "meta",
    "nav",
    "ol",
    "option",
    "p",
    "picture",
    "pre",
    "script",
    "section",
    "select",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "u",
    "ul",
    "video",
];

/// Elements with no closing tag.
const VOID: &[&str] =
    &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];

/// The attributes an element is written with when none are given.
fn default_attributes(tag: &str) -> &'static [&'static str] {
    match tag {
        "a" => &["href"],
        "img" => &["src", "alt"],
        "link" => &["rel=stylesheet", "href"],
        "script" => &[],
        "input" => &["type=text"],
        "label" => &["for"],
        "form" => &["action"],
        "iframe" => &["src"],
        "source" => &["src"],
        _ => &[],
    }
}

/// `text` as it goes into a snippet: `$`, `}` and `\` taken literally.
fn literal(text: &str) -> String {
    text.replace('\\', "\\\\").replace('$', "\\$").replace('}', "\\}")
}

struct Writer {
    text: String,
    /// The last place to fill in numbered so far.
    stop: usize,
    unit: String,
}

impl Writer {
    fn place(&mut self) -> String {
        self.stop += 1;
        format!("${}", self.stop)
    }

    /// Siblings, one a line, each at `depth` levels in from `indent`.
    fn nodes(&mut self, nodes: &[Node], depth: usize, indent: &str) {
        for (i, node) in nodes.iter().enumerate() {
            if i > 0 {
                self.text.push('\n');
                self.text.push_str(indent);
                self.text.push_str(&self.unit.repeat(depth));
            }
            self.node(node, depth, indent);
        }
    }

    fn node(&mut self, node: &Node, depth: usize, indent: &str) {
        let mut open = format!("<{}", literal(&node.tag));
        if let Some(id) = &node.id {
            open.push_str(&format!(" id=\"{}\"", literal(id)));
        }
        if !node.classes.is_empty() {
            open.push_str(&format!(" class=\"{}\"", literal(&node.classes.join(" "))));
        }
        let mut attributes: Vec<(String, Option<String>)> = node.attributes.clone();
        if attributes.is_empty() {
            attributes = default_attributes(&node.tag)
                .iter()
                .map(|a| match a.split_once('=') {
                    Some((k, v)) => (k.to_string(), Some(v.to_string())),
                    None => (a.to_string(), None),
                })
                .collect();
        }
        for (key, value) in attributes {
            let value = match value {
                Some(v) => literal(&v),
                None => self.place(),
            };
            open.push_str(&format!(" {}=\"{value}\"", literal(&key)));
        }
        open.push('>');
        self.text.push_str(&open);
        if VOID.contains(&node.tag.as_str()) {
            return;
        }
        if node.children.is_empty() {
            match &node.text {
                Some(text) => self.text.push_str(&literal(text)),
                None => {
                    let place = self.place();
                    self.text.push_str(&place);
                }
            }
        } else {
            let inner = format!("{indent}{}", self.unit.repeat(depth + 1));
            self.text.push('\n');
            self.text.push_str(&inner);
            if let Some(text) = &node.text {
                self.text.push_str(&literal(text));
                self.text.push('\n');
                self.text.push_str(&inner);
            }
            self.nodes(&node.children, depth + 1, indent);
            self.text.push('\n');
            self.text.push_str(indent);
            self.text.push_str(&self.unit.repeat(depth));
        }
        self.text.push_str(&format!("</{}>", literal(&node.tag)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(abbreviation: &str) -> Option<String> {
        expand_at_end(abbreviation, "", "  ").map(|(_, snippet)| snippet)
    }

    #[test]
    fn abbreviations_become_tags() {
        assert_eq!(expand("div").as_deref(), Some("<div>$1</div>$0"));
        assert_eq!(expand(".card").as_deref(), Some("<div class=\"card\">$1</div>$0"));
        assert_eq!(expand("a").as_deref(), Some("<a href=\"$1\">$2</a>$0"));
        assert_eq!(expand("img").as_deref(), Some("<img src=\"$1\" alt=\"$2\">$0"));
        assert_eq!(
            expand("p#intro.lead.big{Hi there}").as_deref(),
            Some("<p id=\"intro\" class=\"lead big\">Hi there</p>$0")
        );
        assert_eq!(expand(r"code{a\b}").as_deref(), Some(r"<code>a\\b</code>$0"), "taken literally");
        assert_eq!(expand("input[type=email required]").as_deref(), Some("<input type=\"email\" required=\"$1\">$0"));
        assert_eq!(
            expand("ul>li.item$*3").as_deref(),
            Some(
                "<ul>\n  <li class=\"item1\">$1</li>\n  <li class=\"item2\">$2</li>\n  <li class=\"item3\">$3</li>\n</ul>$0"
            )
        );
        assert_eq!(
            expand("header>h1+nav^main").as_deref(),
            Some("<header>\n  <h1>$1</h1>\n  <nav>$2</nav>\n</header>\n<main>$3</main>$0")
        );
    }

    #[test]
    fn only_abbreviations_expand() {
        // Words in a sentence, not elements; a tag being written; nothing.
        assert_eq!(expand("hello"), None);
        assert_eq!(expand_at_end("Some text and", "", "  "), None);
        assert_eq!(expand_at_end("<a href=\"x", "", "  "), None);
        assert_eq!(expand(""), None);
        assert_eq!(expand("ul>"), None);
        assert_eq!(expand("li*0"), None);
        assert_eq!(expand("li*x"), None);
        // After a space or a tag that ends there, only what's after it.
        assert_eq!(expand_at_end("  <p>span.note", "  ", "  ").map(|(start, _)| start), Some(5));
        assert_eq!(expand_at_end("<p>ul>li", "", "  ").map(|(start, _)| start), Some(3));
        assert_eq!(expand_at_end("Read the p", "", "  ").map(|(start, _)| start), Some(9));
        assert_eq!(expand_at_end("x p{a b}", "", "  "), Some((2, "<p>a b</p>$0".into())));
    }

    #[test]
    fn nested_lines_take_the_lines_indentation() {
        let (_, snippet) = expand_at_end("    ul>li", "    ", "\t").unwrap();
        assert_eq!(snippet, "<ul>\n    \t<li>$1</li>\n    </ul>$0");
    }
}
