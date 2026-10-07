//! Tags in HTML and JSX: typing the `>` of an opening tag adds its closing tag, and a
//! tag's name is edited together with its pair's.

use super::{Editor, EditorEvent, Selection};
use gpui::Context;
use std::ops::Range;
use tree_sitter::{Node, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flavor {
    Html,
    Jsx,
}

fn flavor(language: &str) -> Option<Flavor> {
    match language {
        "HTML" => Some(Flavor::Html),
        "JavaScript" | "TSX" => Some(Flavor::Jsx),
        _ => None,
    }
}

/// HTML elements that never have a closing tag.
const VOID: [&str; 14] =
    ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TagKind {
    Opening,
    Closing,
    SelfClosing,
}

/// A tag at the start of some text: its name, and how long it is up to its `>`, in bytes.
#[derive(Debug, PartialEq)]
struct Tag<'a> {
    name: &'a str,
    len: usize,
    kind: TagKind,
}

/// The tag `text` starts with (`<div class="a">`, `</div>`, `<br/>`); None when its `<`
/// starts something else (a comparison...) or the tag isn't finished.
fn tag(text: &str, flavor: Flavor) -> Option<Tag<'_>> {
    let rest = text.strip_prefix('<')?;
    let (rest, closing) = match rest.strip_prefix('/') {
        Some(rest) => (rest, true),
        None => (rest, false),
    };
    let skipped = if closing { 2 } else { 1 };
    let kind = if closing { TagKind::Closing } else { TagKind::Opening };
    let name_len = rest.find(|c: char| !is_name_char(c)).unwrap_or(rest.len());
    let name = &rest[..name_len];
    // `<>` and `</>` are fragments in JSX; otherwise a name starts with a letter.
    match name.chars().next() {
        Some(c) if c.is_alphabetic() => {}
        None if flavor == Flavor::Jsx && rest.starts_with('>') => return Some(Tag { name, len: skipped + 1, kind }),
        _ => return None,
    }
    let mut chars = rest[name_len..].char_indices().peekable();
    // Straight after the name: space or the end of the tag.
    if !matches!(chars.peek(), Some((_, c)) if c.is_whitespace() || *c == '>' || *c == '/' && !closing) {
        return None;
    }
    let len = |at: usize| skipped + name_len + at + 1;
    while let Some((at, c)) = chars.next() {
        match c {
            '>' => return Some(Tag { name, len: len(at), kind }),
            '/' if !closing => {
                let (at, _) = chars.next().filter(|&(_, c)| c == '>')?;
                return Some(Tag { name, len: len(at), kind: TagKind::SelfClosing });
            }
            c if c.is_whitespace() => {}
            // A closing tag is only its name.
            _ if closing => return None,
            '"' | '\'' => {
                chars.find(|&(_, q)| q == c)?;
            }
            '{' if flavor == Flavor::Jsx => {
                let mut depth = 1;
                let mut quote = None;
                while depth > 0 {
                    let (_, c) = chars.next()?;
                    match (quote, c) {
                        (Some(q), c) if c == q => quote = None,
                        (Some(_), _) => {}
                        (None, '"' | '\'' | '`') => quote = Some(c),
                        (None, '{') => depth += 1,
                        (None, '}') => depth -= 1,
                        _ => {}
                    }
                }
            }
            '=' => {}
            c if is_name_char(c) => {}
            _ => return None,
        }
    }
    None
}

/// The name of the opening tag `text` is exactly, from its `<` to its `>`.
fn opening_tag(text: &str, flavor: Flavor) -> Option<&str> {
    tag(text, flavor).filter(|t| t.kind == TagKind::Opening && t.len == text.len()).map(|t| t.name)
}

fn is_void(name: &str, flavor: Flavor) -> bool {
    flavor == Flavor::Html && VOID.contains(&name.to_ascii_lowercase().as_str())
}

/// Whether the `<` at `byte` starts a tag, as the file's syntax sees it: not inside a
/// comment or a string, and in JavaScript not a comparison or a type's `<T>`.
fn starts_a_tag(tree: &Tree, byte: usize, flavor: Flavor) -> bool {
    let Some(node) = tree.root_node().descendant_for_byte_range(byte, byte + 1) else { return false };
    let not_a_tag = |n: &Node| {
        let kind = n.kind();
        kind.contains("comment")
            || kind.contains("string")
            || kind.contains("regex")
            || matches!(kind, "raw_text" | "attribute_value" | "jsx_text")
    };
    let mut ancestor = Some(node);
    while let Some(n) = ancestor {
        if not_a_tag(&n) {
            return false;
        }
        ancestor = n.parent();
    }
    if flavor == Flavor::Jsx
        && node.kind() == "<"
        && node.parent().is_some_and(|p| matches!(p.kind(), "binary_expression" | "type_arguments" | "type_parameters"))
    {
        return false;
    }
    true
}

/// The closing tag to add after `before`, which ends with the `>` just typed. `start` is
/// where `before` starts in the file, in bytes.
fn closing_tag(before: &str, start: usize, tree: &Tree, flavor: Flavor) -> Option<String> {
    if !before.ends_with('>') || before.ends_with("/>") {
        return None;
    }
    for (at, _) in before.rmatch_indices('<').take(64) {
        let Some(name) = opening_tag(&before[at..], flavor) else { continue };
        if !starts_a_tag(tree, start + at, flavor) {
            return None;
        }
        if is_void(name, flavor) {
            return None;
        }
        return Some(format!("</{name}>"));
    }
    None
}

fn name_of(tag: Node<'_>) -> Option<Node<'_>> {
    match tag.kind() {
        "start_tag" | "end_tag" => {
            (0..tag.named_child_count()).filter_map(|i| tag.named_child(i)).find(|c| c.kind() == "tag_name")
        }
        _ => tag.child_by_field_name("name"),
    }
}

fn pair_of(tag: Node<'_>) -> Option<Node<'_>> {
    let parent = tag.parent()?;
    match tag.kind() {
        "jsx_opening_element" => parent.child_by_field_name("close_tag"),
        "jsx_closing_element" => parent.child_by_field_name("open_tag"),
        kind => {
            let wanted = if kind == "start_tag" { "end_tag" } else { "start_tag" };
            (0..parent.child_count()).filter_map(|i| parent.child(i)).find(|c| c.kind() == wanted)
        }
    }
}

/// The innermost tag still open at the end of `before` (which starts at byte `start` of
/// the file): the one `</` closes.
fn innermost_open(before: &str, start: usize, tree: &Tree, flavor: Flavor) -> Option<String> {
    let same = |a: &str, b: &str| if flavor == Flavor::Html { a.eq_ignore_ascii_case(b) } else { a == b };
    let mut open: Vec<&str> = Vec::new();
    let mut at = 0;
    while let Some(found) = before[at..].find('<') {
        let here = at + found;
        if before[here..].starts_with("<!--") {
            at = before[here..].find("-->").map_or(before.len(), |end| here + end + 3);
            continue;
        }
        let Some(tag) = tag(&before[here..], flavor).filter(|_| starts_a_tag(tree, start + here, flavor)) else {
            at = here + 1;
            continue;
        };
        match tag.kind {
            TagKind::Opening if !is_void(tag.name, flavor) => open.push(tag.name),
            TagKind::Closing => {
                if let Some(ix) = open.iter().rposition(|name| same(name, tag.name)) {
                    open.truncate(ix);
                }
            }
            _ => {}
        }
        at = here + tag.len;
    }
    open.pop().map(String::from)
}

/// The name of the tag at `range` (bytes, within the name) and the name of its pair, when
/// both say the same.
fn paired_names(tree: &Tree, text: impl Fn(Range<usize>) -> String, range: Range<usize>) -> Option<[Range<usize>; 2]> {
    // At the very end of a name, the node there is what follows it.
    for at in [range.start, range.start.saturating_sub(1)] {
        let mut node = tree.root_node().descendant_for_byte_range(at, at);
        while let Some(n) = node {
            if matches!(n.kind(), "start_tag" | "end_tag" | "jsx_opening_element" | "jsx_closing_element") {
                break;
            }
            node = n.parent();
        }
        let Some(tag) = node else { continue };
        let Some(name) = name_of(tag) else { continue };
        if !(name.start_byte() <= range.start && range.end <= name.end_byte()) {
            continue;
        }
        let other = pair_of(tag).and_then(name_of)?;
        if text(name.byte_range()) != text(other.byte_range()) {
            return None;
        }
        return Some([name.byte_range(), other.byte_range()]);
    }
    None
}

/// A tag's name and its pair's (chars), after an edit mirrored between them: still paired
/// while the names are emptied or half typed, until something else is edited.
pub(super) struct LinkedTag {
    revision: u64,
    this: Range<usize>,
    other: Range<usize>,
}

impl Editor {
    fn tag_flavor(&self) -> Option<Flavor> {
        flavor(self.language()?.name)
    }

    fn synced_tree(&mut self) -> Option<Tree> {
        let highlighter = self.highlighter.as_mut()?;
        highlighter.sync(&self.buffer);
        highlighter.tree().cloned()
    }

    /// After typing `>` at the end of an opening tag: its closing tag after the caret.
    pub(super) fn close_tag(&mut self, cx: &mut Context<Self>) {
        let Some(flavor) = self.tag_flavor() else { return };
        let caret = self.selection.head;
        if !self.selection.is_empty() || !self.extra.is_empty() {
            return;
        }
        let Some(tree) = self.synced_tree() else { return };
        let from = caret.saturating_sub(4000);
        let before = self.buffer.slice(from..caret);
        let start = self.buffer.rope().char_to_byte(from);
        let Some(closing) = closing_tag(&before, start, &tree, flavor) else { return };
        let len = closing.chars().count();
        if self.buffer.slice(caret..(caret + len).min(self.buffer.len_chars())) == closing {
            return;
        }
        self.buffer.replace(caret..caret, &closing);
        self.selection = Selection::caret(caret);
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
    }

    /// After typing the `/` of `</`: the name of the tag it closes, and its `>`.
    pub(super) fn finish_closing_tag(&mut self, cx: &mut Context<Self>) {
        let Some(flavor) = self.tag_flavor() else { return };
        let caret = self.selection.head;
        if !self.selection.is_empty() || !self.extra.is_empty() || caret < 2 {
            return;
        }
        if self.buffer.slice(caret - 2..caret) != "</" {
            return;
        }
        let Some(tree) = self.synced_tree() else { return };
        let from = caret.saturating_sub(20_000);
        let before = self.buffer.slice(from..caret - 2);
        let start = self.buffer.rope().char_to_byte(from);
        let Some(name) = innermost_open(&before, start, &tree, flavor) else { return };
        let rest = format!("{name}>");
        let len = rest.chars().count();
        if self.buffer.slice(caret..(caret + len).min(self.buffer.len_chars())) == rest {
            return;
        }
        self.buffer.replace(caret..caret, &rest);
        self.selection = Selection::caret(caret + len);
        self.text_changed(cx);
        cx.emit(EditorEvent::Edited);
    }

    /// With the caret in a tag's name (and nothing selected): that name and its pair's, to
    /// outline them.
    pub fn tag_pair_at_caret(&mut self) -> Option<[Range<usize>; 2]> {
        if !self.selection.is_empty() {
            return None;
        }
        let caret = self.selection.head;
        let names = self.linked_tag(&(caret..caret))?;
        names.iter().all(|r| !r.is_empty()).then_some(names)
    }

    /// The tag name `range` (chars) is within, and its pair's, when editing one should
    /// edit the other.
    pub(super) fn linked_tag(&mut self, range: &Range<usize>) -> Option<[Range<usize>; 2]> {
        if !self.extra.is_empty() || self.batch.is_some() {
            return None;
        }
        let within = |name: &Range<usize>| name.start <= range.start && range.end <= name.end;
        if let Some(linked) = &self.linked
            && linked.revision == self.buffer.revision()
        {
            if within(&linked.this) {
                return Some([linked.this.clone(), linked.other.clone()]);
            }
            if within(&linked.other) {
                return Some([linked.other.clone(), linked.this.clone()]);
            }
        }
        self.tag_flavor()?;
        let tree = self.synced_tree()?;
        let rope = self.buffer.rope();
        let bytes = rope.char_to_byte(range.start)..rope.char_to_byte(range.end);
        let [this, other] = paired_names(&tree, |r| rope.byte_slice(r).to_string(), bytes)?;
        let chars = |r: Range<usize>| rope.byte_to_char(r.start)..rope.byte_to_char(r.end);
        Some([chars(this), chars(other)])
    }

    /// Once `edited` (chars, now holding `inserted` chars) changed the name `this`: the same
    /// name in `other`. Returns where the caret at `caret` is now.
    pub(super) fn mirror_tag(
        &mut self,
        [this, other]: [Range<usize>; 2],
        edited: Range<usize>,
        inserted: usize,
        caret: usize,
    ) -> usize {
        let delta = inserted as isize - edited.len() as isize;
        let moved = |p: usize, after: usize, by: isize| if p >= after { (p as isize + by) as usize } else { p };
        let this = this.start..moved(this.end, edited.end, delta).max(this.start);
        let name = self.buffer.slice(this.clone());
        if !name.chars().all(is_name_char) {
            self.linked = None;
            return caret;
        }
        let other = moved(other.start, edited.end, delta)..moved(other.end, edited.end, delta);
        self.buffer.replace(other.clone(), &name);
        let by = name.chars().count() as isize - other.len() as isize;
        let (this, caret) = if other.end <= this.start {
            (moved(this.start, other.end, by)..moved(this.end, other.end, by), moved(caret, other.end, by))
        } else {
            (this, caret)
        };
        self.linked = Some(LinkedTag {
            revision: self.buffer.revision(),
            other: other.start..other.start + name.chars().count(),
            this,
        });
        caret
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The caret in a tag's name: that name and its pair's, from either end; nothing elsewhere.
    #[gpui::test]
    fn the_caret_s_tag_and_its_pair(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext as _;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
        });
        let text = "<div class=\"a\"><p>Hi</p></div>";
        let e = cx.new(|cx| Editor::new(crate::buffer::Buffer::from_text(text), Some("a.html".into()), cx));
        let pair = |cx: &mut gpui::TestAppContext, at: usize| {
            e.update(cx, |e, _| {
                e.selection = Selection::caret(at);
                e.tag_pair_at_caret()
            })
        };
        assert_eq!(pair(cx, 2), Some([1..4, 26..29]), "in <div");
        assert_eq!(pair(cx, 27), Some([26..29, 1..4]), "in </div>");
        assert_eq!(pair(cx, 19), None, "in the text");
        assert_eq!(pair(cx, 7), None, "in an attribute");
    }

    fn tree(text: &str, flavor: Flavor) -> Tree {
        let language: tree_sitter::Language = match flavor {
            Flavor::Html => tree_sitter_html::LANGUAGE.into(),
            Flavor::Jsx => tree_sitter_javascript::LANGUAGE.into(),
        };
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        parser.parse(text, None).unwrap()
    }

    /// What typing the `>` before `|` (or at the end) in `text` closes, if anything.
    fn closes(text: &str, flavor: Flavor) -> Option<String> {
        let caret = text.find('|').unwrap_or(text.len());
        let text = text.replacen('|', "", 1);
        closing_tag(&text[..caret], 0, &tree(&text, flavor), flavor)
    }

    #[test]
    fn closes_html_tags_but_not_void_or_self_closing_ones() {
        let html = Flavor::Html;
        assert_eq!(closes("<div>", html).as_deref(), Some("</div>"));
        assert_eq!(closes("<ul>\n  <li class=\"a > b\">", html).as_deref(), Some("</li>"));
        assert_eq!(closes("<my-widget data-x='1'>", html).as_deref(), Some("</my-widget>"));
        assert_eq!(closes("<br>", html), None);
        assert_eq!(closes("<IMG src=\"a.png\">", html), None);
        assert_eq!(closes("<p/>", html), None);
        assert_eq!(closes("<div></div>", html), None);
        assert_eq!(closes("<!-- <div>| -->", html), None);
        assert_eq!(closes("<script>if (a <b>|) {}</script>", html), None);
    }

    #[test]
    fn closes_jsx_tags_but_not_comparisons() {
        let jsx = Flavor::Jsx;
        assert_eq!(closes("const a = <div>", jsx).as_deref(), Some("</div>"));
        assert_eq!(closes("return (\n  <List.Item onClick={() => go(\"}\")}>", jsx).as_deref(), Some("</List.Item>"));
        assert_eq!(closes("const a = <>", jsx).as_deref(), Some("</>"));
        assert_eq!(closes("const a = <a><b>", jsx).as_deref(), Some("</b>"));
        assert_eq!(closes("if (a <b && c>", jsx), None);
        assert_eq!(closes("const x = a<b>", jsx), None);
        assert_eq!(closes("// <div>", jsx), None);
        assert_eq!(closes("const s = \"<div>|\";", jsx), None);
    }

    #[test]
    fn reads_every_kind_of_tag() {
        let html = Flavor::Html;
        assert_eq!(tag("<div a=\"1\">x", html), Some(Tag { name: "div", len: 11, kind: TagKind::Opening }));
        assert_eq!(tag("</div >x", html), Some(Tag { name: "div", len: 7, kind: TagKind::Closing }));
        assert_eq!(tag("<br/>", html), Some(Tag { name: "br", len: 5, kind: TagKind::SelfClosing }));
        assert_eq!(tag("</>", Flavor::Jsx), Some(Tag { name: "", len: 3, kind: TagKind::Closing }));
        assert_eq!(tag("< b", html), None);
        assert_eq!(tag("<div", html), None);
    }

    /// What `</` typed at the end of `text` closes.
    fn closes_with_slash(text: &str, flavor: Flavor) -> Option<String> {
        let full = format!("{text}</");
        innermost_open(text, 0, &tree(&full, flavor), flavor)
    }

    #[test]
    fn a_slash_closes_the_innermost_open_tag() {
        let html = Flavor::Html;
        assert_eq!(closes_with_slash("<ul>\n  <li>one</li>\n  <li>two", html).as_deref(), Some("li"));
        assert_eq!(closes_with_slash("<ul>\n  <li>one</li>\n  <br><img src=x>\n", html).as_deref(), Some("ul"));
        assert_eq!(closes_with_slash("<div><!-- <p> --><span/>", html).as_deref(), Some("div"));
        assert_eq!(closes_with_slash("<p></p>", html), None);
        let jsx = Flavor::Jsx;
        assert_eq!(closes_with_slash("const a = <List>{xs.map(x => <Item key={x} />)}", jsx).as_deref(), Some("List"));
        assert_eq!(closes_with_slash("const a = <>{a < b}", jsx).as_deref(), Some(""));
    }

    #[test]
    fn a_type_in_tsx_is_no_tag() {
        let text = "const [a, setA] = useState<string>();";
        let caret = text.find('(').unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()).unwrap();
        let tree = parser.parse(text, None).unwrap();
        assert_eq!(closing_tag(&text[..caret], 0, &tree, Flavor::Jsx), None);
    }

    fn editor<'a>(
        cx: &'a mut gpui::TestAppContext,
        name: &str,
        text: &str,
    ) -> (gpui::Entity<Editor>, &'a mut gpui::VisualTestContext) {
        use gpui::Focusable;
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
            crate::keymap::register(crate::keymap::Keymap::Null, cx);
        });
        let (name, text) = (std::path::PathBuf::from(name), text.to_string());
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(crate::buffer::Buffer::from_text(&text), Some(name), cx));
        e.update_in(cx, |e, window, cx| {
            window.focus(&e.focus_handle(cx));
            e.selection = Selection::caret(e.buffer.len_chars());
        });
        (e, cx)
    }

    fn type_keys(cx: &mut gpui::VisualTestContext, text: &str) {
        for c in text.chars() {
            cx.simulate_input(&c.to_string());
        }
    }

    /// `<div>` gets its `</div>`; renaming one renames the other, even through an empty
    /// name; ⌘Z takes it back step by step, the two names alike at every step.
    #[gpui::test]
    fn tags_close_and_rename_together(cx: &mut gpui::TestAppContext) {
        let (e, cx) = editor(cx, "page.html", "");
        type_keys(cx, "<div>");
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "<div></div>");
            assert_eq!(e.selection.head, 5);
        });
        type_keys(cx, "<br>");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "<div><br></div>"));

        // Rename the opening tag: the closing one follows, the caret stays in the name.
        e.update(cx, |e, _| e.selection = Selection { anchor: 1, head: 4 });
        type_keys(cx, "sec");
        cx.simulate_keystrokes("backspace backspace backspace");
        type_keys(cx, "main");
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "<main><br></main>");
            assert_eq!(e.selection.head, 5);
        });
        // And from the closing tag, which moves the caret along with the opening one.
        e.update(cx, |e, _| e.selection = Selection::caret(16));
        type_keys(cx, "-x");
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "<main-x><br></main-x>");
            assert_eq!(e.selection.head, 20);
        });
        // A space ends the name: it isn't copied.
        e.update(cx, |e, _| e.selection = Selection::caret(7));
        type_keys(cx, " id");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "<main-x id><br></main-x>"));
        for _ in 0..12 {
            cx.simulate_keystrokes("cmd-z");
            e.update(cx, |e, _| {
                let text = e.buffer.to_string();
                let opening = text.trim_start_matches('<').split(['>', ' ']).next().unwrap_or("");
                let closing = text.rsplit("</").next().unwrap_or("").trim_end_matches('>');
                assert!(text.is_empty() || opening == closing, "{text}");
            });
        }
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), ""));
    }

    #[gpui::test]
    fn typing_a_closing_slash_names_the_tag(cx: &mut gpui::TestAppContext) {
        let (e, cx) = editor(cx, "page.html", "<section>\n  <p>Hi");
        type_keys(cx, "</");
        e.update(cx, |e, _| {
            assert_eq!(e.buffer.to_string(), "<section>\n  <p>Hi</p>");
            assert_eq!(e.selection.head, e.buffer.len_chars());
        });
        type_keys(cx, "\n</");
        e.update(cx, |e, _| assert_eq!(e.buffer.to_string(), "<section>\n  <p>Hi</p>\n</section>"));
    }

    #[gpui::test]
    fn jsx_closes_tags_but_not_comparisons(cx: &mut gpui::TestAppContext) {
        let (e, cx) = editor(
            cx,
            "App.jsx",
            "const ok = a < b;
const el = ",
        );
        type_keys(cx, "<Row>");
        e.update(cx, |e, _| {
            assert_eq!(
                e.buffer.to_string(),
                "const ok = a < b;
const el = <Row></Row>"
            )
        });
        e.update(cx, |e, _| e.selection = Selection::caret(16));
        type_keys(cx, ">");
        e.update(cx, |e, _| {
            assert_eq!(
                e.buffer.to_string(),
                "const ok = a < b>;
const el = <Row></Row>"
            )
        });
    }

    #[test]
    fn finds_the_paired_name() {
        let pairs = |text: &str, flavor: Flavor, at: usize| {
            paired_names(&tree(text, flavor), |r| text[r].to_string(), at..at)
                .map(|[a, b]| (text[a].to_string(), b.start))
        };
        let html = "<div class=\"x\"><p>hi</p></div>";
        assert_eq!(pairs(html, Flavor::Html, 2), Some(("div".into(), 26)));
        // Right after the name still counts; inside the attributes doesn't.
        assert_eq!(pairs(html, Flavor::Html, 4), Some(("div".into(), 26)));
        assert_eq!(pairs(html, Flavor::Html, 7), None);
        assert_eq!(pairs(html, Flavor::Html, 26), Some(("div".into(), 1)));
        let jsx = "const a = <Box.Row a={1}>x</Box.Row>;";
        assert_eq!(pairs(jsx, Flavor::Jsx, 13), Some(("Box.Row".into(), 28)));
        assert_eq!(pairs(jsx, Flavor::Jsx, 30), Some(("Box.Row".into(), 11)));
        // Names that already differ aren't tied together.
        assert_eq!(pairs("<b>x</i>", Flavor::Html, 1), None);
    }
}
