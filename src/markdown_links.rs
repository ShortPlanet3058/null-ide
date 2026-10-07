//! Markdown links follow files that move: renaming or moving a file (or a folder) in Null
//! rewrites the links and images that point at it, in the project's Markdown files, and a
//! moved Markdown file's own links to what stayed put. No language server does this for
//! Markdown; these edits go through the same path as a server's.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// Projects are looked through up to this many Markdown files, this big at most.
const MAX_FILES: usize = 5_000;
const MAX_BYTES: u64 = 1024 * 1024;

/// A link's target as written, where it sits in its line (bytes), and the line.
pub struct Target<'a> {
    pub line: usize,
    pub start: usize,
    pub text: &'a str,
}

/// The byte ranges of a line's inline code spans (`` `like this` ``): links there are text.
pub fn code_spans(line: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(open) = line[from..].find('`') {
        let open = from + open;
        let ticks = line[open..].chars().take_while(|c| *c == '`').count();
        let fence = "`".repeat(ticks);
        match line[open + ticks..].find(&fence) {
            Some(close) => {
                let end = open + ticks + close + ticks;
                spans.push(open..end);
                from = end;
            }
            None => break,
        }
    }
    spans
}

/// The local targets of a Markdown text's links and images (`[a](b)`, `![a](b)`,
/// `[a](<b c>)`, `[a]: b`, `src="b"`), not addresses nor `#headings`, nor what's in code
/// (fences, `inline code`) or a footnote. Lines as the editor counts them (a lone \r ends
/// one too), so line numbers are the editor's.
pub fn targets(text: &str) -> Vec<Target<'_>> {
    let mut found = Vec::new();
    let mut fence: Option<&str> = None;
    for (line, l) in crate::markdown_view::buffer_lines(text).into_iter().enumerate() {
        if let Some(open) = crate::markdown_view::fence_of(l.trim_start()) {
            fence = if fence == Some(open) { None } else { fence.or(Some(open)) };
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let code = code_spans(l);
        let mut add = |start: usize, end: usize| {
            let text = &l[start..end];
            let local = !text.is_empty() && !text.starts_with('#') && !text.contains("://") && !text.contains(':');
            if local && !code.iter().any(|c| c.contains(&start)) {
                found.push(Target { line, start, text });
            }
        };
        // [text](target "title"), ![alt](target), [text](<target with spaces>)
        let mut from = 0;
        while let Some(at) = l[from..].find("](") {
            let start = from + at + 2;
            if l[start..].starts_with('<') {
                if let Some(close) = l[start + 1..].find('>') {
                    add(start + 1, start + 1 + close);
                }
            } else {
                let end = l[start..].find([')', ' ', '\t']).map_or(l.len(), |e| start + e);
                add(start, end);
            }
            from = start;
        }
        // [label]: target (not a footnote's [^label]: text)
        let trimmed = l.trim_start();
        if trimmed.starts_with('[')
            && !trimmed.starts_with("[^")
            && let Some(close) = trimmed.find("]:")
        {
            let rest = &trimmed[close + 2..];
            let lead = rest.len() - rest.trim_start().len();
            let start = l.len() - trimmed.len() + close + 2 + lead;
            let end = l[start..].find([' ', '\t']).map_or(l.len(), |e| start + e);
            add(start, end);
        }
        // <img src="target"> and the like
        for quote in ["src=\"", "href=\""] {
            let mut from = 0;
            while let Some(at) = l[from..].find(quote) {
                let start = from + at + quote.len();
                let end = l[start..].find('"').map_or(l.len(), |e| start + e);
                add(start, end);
                from = end;
            }
        }
    }
    found
}

/// A link's path as a file name: `%20` and other `%XX` read as what they stand for, a
/// `?query` or `#section` left off.
pub fn decoded(path: &str) -> String {
    let path = path.split(['#', '?']).next().unwrap_or("");
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(a), Some(b)) = (bytes.get(i + 1).and_then(|b| hex(*b)), bytes.get(i + 2).and_then(|b| hex(*b)))
        {
            out.push((a * 16 + b) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| path.to_string())
}

/// `path` with `.` and `..` worked out, without asking the disk (the file may be gone).
pub fn tidy(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// How to write `to` from the folder `dir`: `img/a.png`, `../docs/b.md`.
fn relative(dir: &Path, to: &Path) -> String {
    let (dir, to): (Vec<_>, Vec<_>) = (dir.components().collect(), to.components().collect());
    let shared = dir.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); dir.len() - shared];
    parts.extend(to[shared..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    parts.join("/")
}

/// Where `path` is once `from` has moved to `to` (it or something inside it); None if it
/// isn't affected.
fn moved(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    path.strip_prefix(from).ok().map(|rest| if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) })
}

/// The new target for a link written `written` in the Markdown file `file`, once `from`
/// moves to `to`; None when it stays as it is.
fn new_target(written: &str, file: &Path, from: &Path, to: &Path) -> Option<String> {
    // `?query` and `#section` kept as they are, after the path.
    let (path_part, fragment) = written.split_at(written.find(['?', '#']).unwrap_or(written.len()));
    let encoded = path_part.contains("%20");
    let decoded = decoded(path_part);
    let dir = file.parent()?;
    let target = if decoded.starts_with('/') { PathBuf::from(&decoded) } else { tidy(&dir.join(&decoded)) };
    // The file itself may be what moved (or be in what moved): it's read from its new place.
    let new_dir =
        moved(file, from, to).and_then(|f| f.parent().map(Path::to_path_buf)).unwrap_or_else(|| dir.to_path_buf());
    let new_target = moved(&target, from, to).unwrap_or_else(|| target.clone());
    if new_dir == dir && new_target == target {
        return None;
    }
    // Written from the root as it was (`/docs/a.md`): kept that way.
    let mut rewritten = if decoded.starts_with('/') {
        new_target.to_string_lossy().into_owned()
    } else {
        let mut r = relative(&new_dir, &new_target);
        if decoded.starts_with("./") && !r.starts_with("..") {
            r = format!("./{r}");
        }
        r
    };
    if encoded {
        rewritten = rewritten.replace(' ', "%20");
    }
    let rewritten = format!("{rewritten}{fragment}");
    (rewritten != written).then_some(rewritten)
}

/// The edits that keep the project's Markdown links right once `from` moves to `to`, as a
/// language server would send them. Files open in Null are read from `open` (their text in
/// the editor, saved or not), the others from disk.
// A workspace edit is keyed by `Uri` (lsp_types' choice), which clippy calls mutable.
#[allow(clippy::mutable_key_type)]
pub fn edits_for_move(
    root: &Path,
    from: &Path,
    to: &Path,
    open: &HashMap<PathBuf, String>,
) -> Option<lsp_types::WorkspaceEdit> {
    let mut changes: HashMap<lsp_types::Uri, Vec<lsp_types::TextEdit>> = HashMap::new();
    let markdown = ignore::WalkBuilder::new(root)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| e.path().extension().is_some_and(|x| x == "md" || x == "markdown" || x == "mdx"))
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_BYTES))
        .take(MAX_FILES);
    for entry in markdown {
        let file = entry.path();
        let Some(text) = open.get(file).cloned().or_else(|| std::fs::read_to_string(file).ok()) else { continue };
        let lines: Vec<&str> = crate::markdown_view::buffer_lines(&text);
        let edits: Vec<lsp_types::TextEdit> = targets(&text)
            .into_iter()
            .filter_map(|t| {
                let line = lines[t.line];
                let new = new_target(t.text, file, from, to)?;
                // A space reads as the end of a link unless it's in <…>.
                let bracketed = line[..t.start].ends_with('<');
                let new = if bracketed { new } else { new.replace(' ', "%20") };
                let column = |byte: usize| line[..byte].encode_utf16().count() as u32;
                let start = lsp_types::Position { line: t.line as u32, character: column(t.start) };
                let end = lsp_types::Position { line: t.line as u32, character: column(t.start + t.text.len()) };
                Some(lsp_types::TextEdit { range: lsp_types::Range { start, end }, new_text: new })
            })
            .collect();
        if !edits.is_empty()
            && let Some(uri) = crate::lsp::uri_for(file)
        {
            changes.insert(uri, edits);
        }
    }
    (!changes.is_empty()).then(|| lsp_types::WorkspaceEdit { changes: Some(changes), ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(text: &str) -> Vec<&str> {
        targets(text).into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn targets_are_read_as_written() {
        // <…> around a path with spaces (as Null writes it), a footnote, code, a fence, and
        // a lone \r ending a line: only the links count, on the editor's lines.
        let text = "![a](<My Shot.png>) `[x](no.md)` [^1]: Some note\r[b](b.md)\n```\n[c](c.md)\n```\n";
        let found: Vec<(usize, &str)> = targets(text).into_iter().map(|t| (t.line, t.text)).collect();
        assert_eq!(found, [(0, "My Shot.png"), (1, "b.md")]);
        assert_eq!(decoded("caf%C3%A9.png?raw=1#top"), "café.png");
    }

    #[test]
    fn finds_local_targets() {
        let text = "See [the guide](docs/guide.md#setup \"Guide\") and ![shot](img/a%20b.png).\n\
                    [ref]: ../notes.md\n<img src=\"logo.svg\"> [web](https://x.dev) [top](#top)\r\n";
        assert_eq!(written(text), ["docs/guide.md#setup", "img/a%20b.png", "../notes.md", "logo.svg"]);
    }

    #[test]
    fn links_follow_what_moved() {
        let (from, to) = (Path::new("/p/img/a b.png"), Path::new("/p/assets/a b.png"));
        let readme = Path::new("/p/README.md");
        assert_eq!(new_target("img/a%20b.png", readme, from, to).as_deref(), Some("assets/a%20b.png"));
        assert_eq!(new_target("./img/a%20b.png", readme, from, to).as_deref(), Some("./assets/a%20b.png"));
        assert_eq!(new_target("other.png", readme, from, to), None);
        // A folder moved: what's inside it follows, its #fragment kept.
        let (from, to) = (Path::new("/p/docs"), Path::new("/p/guide"));
        assert_eq!(new_target("docs/setup.md#run", readme, from, to).as_deref(), Some("guide/setup.md#run"));
        // A Markdown file moved: its own links to what stayed are written from its new place.
        let (from, to) = (Path::new("/p/docs/setup.md"), Path::new("/p/setup.md"));
        assert_eq!(new_target("../img/a.png", from, from, to).as_deref(), Some("img/a.png"));
        assert_eq!(new_target("other.md", Path::new("/p/docs/other.md"), from, to).as_deref(), None);
        assert_eq!(new_target("setup.md", Path::new("/p/docs/other.md"), from, to).as_deref(), Some("../setup.md"));
    }
}
