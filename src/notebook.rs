//! Jupyter notebooks (`.ipynb`) as they read: their JSON turned into Markdown for the
//! preview (⌘⇧V). Text cells as written, code in its language, and what each cell printed
//! below it (an error's message without the terminal's colours, a chart as its picture).
//!
//! And as they're written: their cells as one text, each after a `# %%` line (Markdown
//! ones `# %% [markdown]`, their lines behind `# `), as Jupytext and VS Code write them; it
//! goes back into the notebook on saving, each cell keeping what it printed, its id and
//! its settings.

use serde_json::Value;

/// Whether `json` reads as a notebook (it has cells), without turning it into anything:
/// asked on opening each one, where its outputs (pictures) can be many megabytes.
pub fn is_notebook(json: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Notebook {
        #[allow(dead_code)]
        cells: Vec<serde::de::IgnoredAny>,
    }
    serde_json::from_str::<Notebook>(json).is_ok()
}

/// The notebook's language (its kernel's), and how a line is said to be a comment in it.
fn language_of(notebook: &Value) -> (String, &'static str) {
    let metadata = &notebook["metadata"];
    let language = metadata["kernelspec"]["language"]
        .as_str()
        .or(metadata["language_info"]["name"].as_str())
        .unwrap_or("python")
        .to_lowercase();
    let comment = match language.as_str() {
        "python" | "r" | "julia" | "ruby" | "perl" | "bash" | "sh" | "powershell" => "#",
        "sql" | "haskell" | "lua" => "--",
        "matlab" | "octave" => "%",
        _ => "//",
    };
    (language, comment)
}

/// A file's text as it's searched and written in: a notebook's, its cells (see
/// `to_cells_text`); any other's, as it is.
pub fn cells_of_file(path: &std::path::Path, text: String) -> String {
    let notebook = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ipynb"));
    match notebook.then(|| to_cells_text(&text)).flatten() {
        Some(cells) => cells,
        None => text,
    }
}

/// The notebook's language as Null names it (`python` is Python), for colouring its cells.
pub fn language(json: &str) -> Option<String> {
    let notebook: Value = serde_json::from_str(json).ok()?;
    let (language, _) = language_of(&notebook);
    let mut name = language.chars();
    Some(name.next().map(|c| c.to_uppercase().chain(name).collect()).unwrap_or_default())
}

/// A cell, as the text has it: its kind (`code`, `markdown`, `raw`) and its text.
type Cell = (String, String);

/// The notebook's cells as one text to write in: each after a `# %%` line (in its
/// language's comment), Markdown and raw ones' lines behind it too. None when `json`
/// isn't a notebook.
pub fn to_cells_text(json: &str) -> Option<String> {
    let notebook: Value = serde_json::from_str(json).ok()?;
    let (_, comment) = language_of(&notebook);
    let mut out = String::new();
    for cell in notebook.get("cells")?.as_array()? {
        let kind = cell["cell_type"].as_str().unwrap_or("code");
        let source = text_of(&cell["source"]);
        if !out.is_empty() {
            out.push('\n');
        }
        match kind {
            "code" => {
                out.push_str(&format!("{comment} %%\n"));
                out.push_str(&source);
            }
            other => {
                out.push_str(&format!("{comment} %% [{other}]\n"));
                let lines: Vec<String> = source
                    .split('\n')
                    .map(|l| if l.is_empty() { comment.to_string() } else { format!("{comment} {l}") })
                    .collect();
                out.push_str(&lines.join("\n"));
            }
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    // Only when the cells come back as they are (a cell's own line that reads as a cell's
    // start would split it): else the notebook stays its JSON, as text.
    let back = cells_in(&out, comment);
    let cells: Vec<Cell> = notebook["cells"]
        .as_array()?
        .iter()
        .map(|c| {
            (
                c["cell_type"].as_str().unwrap_or("code").to_string(),
                text_of(&c["source"]).trim_end_matches('\n').to_string(),
            )
        })
        .collect();
    let back: Vec<Cell> = back.into_iter().map(|(k, t)| (k, t.trim_end_matches('\n').to_string())).collect();
    (back == cells).then_some(out)
}

/// The cells in `text` (as `to_cells_text` writes them, `comment` being the language's).
fn cells_in(text: &str, comment: &str) -> Vec<Cell> {
    let marker = format!("{comment} %%");
    let mut cells: Vec<(String, Vec<&str>)> = Vec::new();
    for line in text.split('\n') {
        let trimmed = line.trim_end();
        // A cell's line: `# %%` alone, or with its kind (`# %% [markdown]`); `# %%timeit`
        // is a cell's own text.
        if let Some(kind) = trimmed.strip_prefix(&marker).and_then(marker_kind) {
            cells.push((kind.to_string(), Vec::new()));
            continue;
        }
        match cells.last_mut() {
            Some((_, lines)) => lines.push(line),
            // Written before the first cell: a cell of code.
            None if !line.trim().is_empty() => cells.push(("code".into(), vec![line])),
            None => {}
        }
    }
    cells
        .into_iter()
        .map(|(kind, mut lines)| {
            // The empty line between cells isn't the cell's (spaces written at its end are).
            while lines.last().is_some_and(|l| l.trim_end_matches('\r').is_empty()) {
                lines.pop();
            }
            let lines: Vec<String> = if kind == "code" {
                lines.iter().map(|l| l.to_string()).collect()
            } else {
                lines
                    .iter()
                    .map(|l| {
                        let l = l.strip_prefix(comment).unwrap_or(l);
                        l.strip_prefix(' ').unwrap_or(l).to_string()
                    })
                    .collect()
            };
            (kind, lines.join("\n"))
        })
        .collect()
}

/// What follows `# %%` on a cell's line: nothing (code), or `[kind]`; None when it's text.
fn marker_kind(rest: &str) -> Option<&str> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Some("code");
    }
    let kind = rest.strip_prefix('[')?.strip_suffix(']')?.trim();
    (!kind.is_empty() && kind.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then_some(kind)
}

/// A cell's source as notebooks keep it: its lines, each with its line break but the last.
fn source_value(text: &str) -> Value {
    let mut lines: Vec<Value> = text.split_inclusive('\n').map(|l| Value::String(l.to_string())).collect();
    if text.is_empty() {
        lines.clear();
    }
    Value::Array(lines)
}

/// The notebook `json` with the cells written in `text` (see `to_cells_text`): a cell
/// still there keeps what it printed, its id and settings (an edited one too, as Jupyter
/// keeps them until it runs again), a new one starts empty. `json` itself when nothing
/// changed; None when it isn't a notebook.
pub fn with_cells(json: &str, text: &str) -> Option<String> {
    let mut notebook: Value = serde_json::from_str(json).ok()?;
    let (_, comment) = language_of(&notebook);
    let old: Vec<Value> = notebook.get("cells")?.as_array()?.clone();
    let new = cells_in(text, comment);
    let as_cell = |c: &Value| -> Cell {
        (
            c["cell_type"].as_str().unwrap_or("code").to_string(),
            text_of(&c["source"]).trim_end_matches('\n').to_string(),
        )
    };
    let old_cells: Vec<Cell> = old.iter().map(as_cell).collect();
    let trimmed = |cells: &[Cell]| -> Vec<Cell> {
        cells.iter().map(|(k, t)| (k.clone(), t.trim_end_matches('\n').to_string())).collect()
    };
    if old_cells == trimmed(&new) {
        return Some(json.to_string());
    }
    // Which old cell each new one is: the same ones in order (the longest run of them),
    // then, between two of those, edited ones taken in turn by kind.
    let (n, m) = (new.len(), old_cells.len());
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if new[i] == old_cells[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let mut was: Vec<Option<usize>> = vec![None; n];
    let (mut i, mut j) = (0, 0);
    let mut anchors = vec![(0usize, 0usize)];
    while i < n && j < m {
        if new[i] == old_cells[j] {
            was[i] = Some(j);
            i += 1;
            j += 1;
            anchors.push((i, j));
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    anchors.push((n, m));
    for pair in anchors.windows(2) {
        let ((new_from, old_from), (new_to, old_to)) = (pair[0], pair[1]);
        let mut free: Vec<usize> = (old_from..old_to.min(m)).filter(|o| !was.contains(&Some(*o))).collect();
        for k in new_from..new_to.min(n) {
            if was[k].is_some() {
                continue;
            }
            if let Some(at) = free.iter().position(|&o| old_cells[o].0 == new[k].0) {
                was[k] = Some(free.remove(at));
            }
        }
    }
    let with_ids = old.iter().any(|c| c.get("id").is_some()) || notebook["nbformat_minor"].as_u64().unwrap_or(0) >= 5;
    let cells: Vec<Value> = new
        .iter()
        .zip(&was)
        .enumerate()
        .map(|(k, ((kind, text), was))| {
            let mut cell = match was.map(|o| &old[o]) {
                Some(cell) if cell["cell_type"].as_str() == Some(kind.as_str()) => cell.clone(),
                _ => {
                    let mut cell = serde_json::Map::new();
                    cell.insert("cell_type".into(), Value::String(kind.clone()));
                    if kind == "code" {
                        cell.insert("execution_count".into(), Value::Null);
                    }
                    if with_ids {
                        cell.insert("id".into(), Value::String(new_id(k, text)));
                    }
                    cell.insert("metadata".into(), Value::Object(Default::default()));
                    if kind == "code" {
                        cell.insert("outputs".into(), Value::Array(Vec::new()));
                    }
                    cell.insert("source".into(), Value::Null);
                    Value::Object(cell)
                }
            };
            cell["source"] = source_value(text);
            cell
        })
        .collect();
    notebook["cells"] = Value::Array(cells);
    // As Jupyter writes them: a space's indent, a line break at the end.
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut writer = serde_json::Serializer::with_formatter(&mut out, formatter);
    serde::Serialize::serialize(&notebook, &mut writer).ok()?;
    let mut out = String::from_utf8(out).ok()?;
    out.push('\n');
    Some(out)
}

/// An id for a new cell: eight hex digits, from where it is and what it says.
fn new_id(at: usize, text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    (at, text, std::time::SystemTime::now()).hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

/// A cell's text: a string, or a list of lines.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(lines) => lines.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

/// `text` without the terminal's colour codes (`\x1b[0;31m`), as tracebacks have them.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// What a progress bar left: each line as it was last rewritten (`\r` goes back to its
/// start).
fn last_drawn(text: &str) -> String {
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            line.rsplit('\r').next().unwrap_or(line)
        })
        .collect();
    lines.join("\n")
}

/// A text cell, closed: a code block or comment it leaves open would take in the cells
/// after it, which Jupyter keeps apart.
fn closed(text: &str) -> String {
    let mut text = text.trim_end().to_string();
    // As the preview reads it: a fence until one that closes it; outside code, a line
    // starting `<!--` until a line with `-->`.
    let mut fence: Option<String> = None;
    let mut comment = false;
    for line in text.lines() {
        let line = line.trim_start();
        if comment {
            comment = !line.contains("-->");
            continue;
        }
        match &fence {
            Some(open) if crate::markdown_view::closes(line, open) => fence = None,
            Some(_) => {}
            None if line.starts_with("<!--") => comment = !line[4..].contains("-->"),
            None => fence = crate::markdown_view::fence_of(line).map(str::to_string),
        }
    }
    if let Some(open) = fence {
        text.push('\n');
        text.push_str(&open);
    } else if comment {
        text.push_str("\n-->");
    }
    text
}

/// A fence that the text inside can't close: longer than any run of backticks in it.
fn fence(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(longest.max(2) + 1)
}

fn code_block(language: &str, text: &str) -> String {
    let text = text.trim_end_matches('\n');
    let fence = fence(text);
    format!("{fence}{language}\n{text}\n{fence}\n\n")
}

/// The notebook in `json` as Markdown; None when it isn't one.
pub fn to_markdown(json: &str) -> Option<String> {
    let notebook: Value = serde_json::from_str(json).ok()?;
    let cells = notebook.get("cells")?.as_array()?;
    let (language, _) = language_of(&notebook);
    let mut out = String::new();
    for cell in cells {
        let source = text_of(&cell["source"]);
        match cell["cell_type"].as_str() {
            Some("markdown") => {
                out.push_str(&closed(&source));
                out.push_str("\n\n");
            }
            Some("code") => {
                out.push_str(&code_block(&language, &source));
                for output in cell["outputs"].as_array().into_iter().flatten() {
                    let shown = match output["output_type"].as_str() {
                        Some("stream") => text_of(&output["text"]),
                        Some("error") => {
                            let traceback = text_of(&Value::Array(
                                output["traceback"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|l| l.as_str().map(|l| Value::String(format!("{l}\n"))))
                                    .collect(),
                            ));
                            if traceback.is_empty() {
                                format!("{}: {}", text_of(&output["ename"]), text_of(&output["evalue"]))
                            } else {
                                traceback
                            }
                        }
                        Some("execute_result" | "display_data") => {
                            let data = &output["data"];
                            // A chart: shown, from the picture the notebook keeps.
                            if let Some((kind, picture)) = ["png", "jpeg", "gif"]
                                .iter()
                                .find_map(|kind| Some((kind, text_of(data.get(format!("image/{kind}").as_str())?))))
                            {
                                let picture: String = picture.chars().filter(|c| !c.is_whitespace()).collect();
                                out.push_str(&format!("![output](data:image/{kind};base64,{picture})\n\n"));
                                continue;
                            }
                            text_of(&data["text/plain"])
                        }
                        _ => continue,
                    };
                    if !shown.trim().is_empty() {
                        out.push_str(&code_block("output", &last_drawn(&plain(&shown))));
                    }
                }
            }
            // Raw cells: as they are, in a block.
            _ if !source.trim().is_empty() => out.push_str(&code_block("text", &source)),
            _ => {}
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A notebook opens as it reads; one that doesn't read as a notebook, as its text.
    #[gpui::test]
    fn a_notebook_opens_as_it_reads(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let dir = crate::tools::test_dir("notebook-open");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cell = r##"{"cell_type":"markdown","metadata":{},"source":["# Hi"]}"##;
        std::fs::write(
            dir.join("a.ipynb"),
            format!(r#"{{"cells":[{cell}],"metadata":{{}},"nbformat":4,"nbformat_minor":5}}"#),
        )
        .unwrap();
        std::fs::write(dir.join("broken.ipynb"), "{ not json").unwrap();
        for (name, reading) in [("a.ipynb", true), ("broken.ipynb", false)] {
            let path = dir.join(name);
            let (e, cx) = cx.add_window_view(|_, cx| crate::editor::Editor::open(path, None, cx));
            assert_eq!(e.read_with(cx, |e, _| e.reading), reading, "{name}");
            // Sent to a match in it: its text, there.
            e.update(cx, |e, cx| e.reveal_match(0, 0..1, Default::default(), cx));
            assert!(!e.read_with(cx, |e, _| e.reading), "{name}: at the match");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A notebook as Jupyter writes it.
    const NOTEBOOK: &str = r##"{
 "cells": [
  {
   "cell_type": "markdown",
   "id": "a1",
   "metadata": {},
   "source": [
    "# Sales\n",
    "\n",
    "By month."
   ]
  },
  {
   "cell_type": "code",
   "execution_count": 1,
   "id": "b2",
   "metadata": {},
   "outputs": [
    {
     "name": "stdout",
     "output_type": "stream",
     "text": [
      "3\n"
     ]
    }
   ],
   "source": [
    "x = 1 + 2\n",
    "print(x)"
   ]
  },
  {
   "cell_type": "code",
   "execution_count": 2,
   "id": "c3",
   "metadata": {},
   "outputs": [],
   "source": [
    "y = x * 2"
   ]
  }
 ],
 "metadata": {
  "kernelspec": {
   "language": "python",
   "name": "python3"
  }
 },
 "nbformat": 4,
 "nbformat_minor": 5
}
"##;

    /// A notebook written in: its cells as text (in Python), saved back into its JSON with
    /// what each cell printed; read again from disk as its cells.
    #[gpui::test]
    fn a_notebook_is_written_in_as_its_cells(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::settings::Settings::default());
            cx.set_global(crate::theme::Theme::oled());
            cx.set_global(crate::fonts::Fonts { code: "Menlo".into(), ui: "Helvetica".into() });
        });
        let dir = crate::tools::test_dir("notebook-write");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sales.ipynb");
        std::fs::write(&path, NOTEBOOK).unwrap();
        let (e, cx) = cx.add_window_view(|_, cx| crate::editor::Editor::open(path.clone(), None, cx));
        e.update(cx, |e, cx| {
            assert!(e.reading, "it opens as it reads");
            assert!(e.buffer.to_string().starts_with("# %% [markdown]"));
            assert_eq!(e.language_name(), "Python");
            let at = e.buffer.to_string().find("1 + 2").unwrap();
            e.buffer.replace(at..at + 1, "40");
            assert!(e.save_to_disk(cx));
        });
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["cells"][1]["source"][0], "x = 40 + 2\n");
        assert_eq!(saved["cells"][1]["outputs"][0]["text"][0], "3\n", "what it printed, kept");
        // Open on both sides: the other copy writes cells into the JSON too.
        let twin_source = e.read_with(cx, |e, _| e.twin_source().unwrap());
        let (twin, cx) = cx.add_window_view(|_, cx| crate::editor::Editor::twin(twin_source, None, cx));
        twin.update(cx, |t, cx| {
            let at = t.buffer.to_string().find("y = x").unwrap();
            t.buffer.replace(at..at, "# twice\n");
            assert!(t.save_to_disk(cx));
        });
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).expect("still a notebook");
        assert_eq!(saved["cells"][2]["source"][0], "# twice\n");
        // Compared with the notebook as it was (File History, Compare with Saved): its cells.
        twin.update(cx, |t, cx| {
            t.start_review(NOTEBOOK.to_string(), cx);
            assert!(t.in_review(), "the change to take back");
        });
        // Renamed (another kind of name): still a notebook on disk, written back as one;
        // saved as a script (Save As .py): the cells, as they are.
        e.update(cx, |e, cx| {
            let renamed = dir.join("sales.ipynb.bak");
            std::fs::rename(&path, &renamed).unwrap();
            e.set_path(renamed.clone(), None, cx);
            assert!(e.notebook.is_some(), "a rename keeps it a notebook");
            std::fs::rename(&renamed, &path).unwrap();
            e.set_path(path.clone(), None, cx);
            let script = dir.join("sales.py");
            e.saving_as(&script);
            e.set_path(script.clone(), None, cx);
            assert!(e.save_to_disk(cx));
            assert!(std::fs::read_to_string(&script).unwrap().starts_with("# %% [markdown]"));
            e.saving_as(&path);
            e.set_path(path.clone(), None, cx);
        });
        // Changed elsewhere (Jupyter): read again as its cells.
        let elsewhere = NOTEBOOK.replace("y = x * 2", "y = x * 3");
        std::fs::write(&path, &elsewhere).unwrap();
        e.update(cx, |e, cx| {
            e.reload_from_disk(cx);
            assert!(e.buffer.to_string().ends_with("# %%\ny = x * 3\n"), "{}", e.buffer.to_string());
            assert!(!e.buffer.is_dirty());
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cells_as_text_to_write_in() {
        let text = to_cells_text(NOTEBOOK).unwrap();
        assert_eq!(
            text,
            "# %% [markdown]\n# # Sales\n#\n# By month.\n\n# %%\nx = 1 + 2\nprint(x)\n\n# %%\ny = x * 2\n"
        );
        assert_eq!(language(NOTEBOOK).as_deref(), Some("Python"));
        // Nothing changed: the notebook as it was, to the byte.
        assert_eq!(with_cells(NOTEBOOK, &text).unwrap(), NOTEBOOK);
        let cells = |json: &str| -> Vec<Value> {
            serde_json::from_str::<Value>(json).unwrap()["cells"].as_array().unwrap().clone()
        };
        // A cell edited keeps what it printed and its id; the text's line breaks as Jupyter's.
        let edited = with_cells(NOTEBOOK, &text.replace("x = 1 + 2", "x = 40 + 2")).unwrap();
        let after = cells(&edited);
        assert_eq!(after[1]["id"], "b2");
        assert_eq!(after[1]["outputs"][0]["text"][0], "3\n");
        assert_eq!(after[1]["source"], serde_json::json!(["x = 40 + 2\n", "print(x)"]));
        assert!(edited.starts_with("{\n \"cells\": [\n  {\n   \"cell_type\": \"markdown\""), "Jupyter's layout");
        // A new cell in between: empty, its own id; the others keep theirs and their outputs.
        let added = with_cells(NOTEBOOK, &text.replace("# %%\ny = x", "# %%\nz = 0\n\n# %%\ny = x")).unwrap();
        let after = cells(&added);
        assert_eq!(after.len(), 4);
        assert_eq!((after[1]["id"].as_str(), after[3]["id"].as_str()), (Some("b2"), Some("c3")));
        assert_eq!(after[2]["outputs"], serde_json::json!([]));
        assert_eq!(after[2]["execution_count"], Value::Null);
        assert!(after[2]["id"].as_str().is_some_and(|id| id.len() == 8));
        // A cell taken out: gone, the rest as they were.
        let removed = with_cells(NOTEBOOK, &text.replace("# %%\nx = 1 + 2\nprint(x)\n\n", "")).unwrap();
        let after = cells(&removed);
        assert_eq!(after.iter().map(|c| c["id"].as_str().unwrap()).collect::<Vec<_>>(), ["a1", "c3"]);
        // Markdown edited: its lines without their comment.
        let words = with_cells(NOTEBOOK, &text.replace("# By month.", "# By month and year.")).unwrap();
        assert_eq!(cells(&words)[0]["source"], serde_json::json!(["# Sales\n", "\n", "By month and year."]));
        assert!(with_cells("{ not json", &text).is_none());
        // Spaces taken off a cell's end: saved too (not "nothing changed").
        let spaced = NOTEBOOK.replace("\"y = x * 2\"", "\"y = x * 2  \"");
        let text = to_cells_text(&spaced).unwrap();
        let tidied = with_cells(&spaced, &text.replace("y = x * 2  ", "y = x * 2")).unwrap();
        assert_eq!(cells(&tidied)[2]["source"], serde_json::json!(["y = x * 2"]));
        // A cell's own `# %%timeit` (a cell magic, commented) stays its text.
        let timed = NOTEBOOK.replace("\"y = x * 2\"", "\"# %%timeit\\n\", \"y = x * 2\"");
        let text = to_cells_text(&timed).unwrap();
        assert!(text.contains("# %%\n# %%timeit\ny = x * 2"), "{text}");
        assert_eq!(with_cells(&timed, &text).unwrap(), timed);
        // A cell whose own line is a cell's start: can't be written as cells, stays JSON.
        let split = NOTEBOOK.replace("\"y = x * 2\"", "\"a = 1\\n\", \"# %%\\n\", \"b = 2\"");
        assert!(to_cells_text(&split).is_none());
    }

    #[test]
    fn a_notebook_reads_as_markdown() {
        let json = r##"{
          "metadata": {"kernelspec": {"language": "python", "name": "python3"}},
          "nbformat": 4,
          "cells": [
            {"cell_type": "markdown", "source": ["# Sales\n", "By *month*."]},
            {"cell_type": "code", "source": "total = 3 + 4\nprint(total)", "outputs": [
              {"output_type": "stream", "name": "stdout", "text": ["7\n"]}
            ]},
            {"cell_type": "code", "source": ["df.plot()"], "outputs": [
              {"output_type": "display_data", "data": {"image/png": "iVBOR\nw0=", "text/plain": ["<Figure>"]}}
            ]},
            {"cell_type": "code", "source": ["1/0"], "outputs": [
              {"output_type": "error", "ename": "ZeroDivisionError", "evalue": "division by zero",
               "traceback": ["\u001b[0;31mZeroDivisionError\u001b[0m: division by zero"]}
            ]}
          ]
        }"##;
        let markdown = to_markdown(json).unwrap();
        assert_eq!(
            markdown,
            "# Sales\nBy *month*.\n\n```python\ntotal = 3 + 4\nprint(total)\n```\n\n```output\n7\n```\n\n```python\ndf.plot()\n```\n\n![output](data:image/png;base64,iVBORw0=)\n\n```python\n1/0\n```\n\n```output\nZeroDivisionError: division by zero\n```\n\n"
        );
    }

    #[test]
    fn cells_stay_apart() {
        let json = r#"{"metadata": {}, "cells": [
            {"cell_type": "markdown", "source": "```python\nnot closed"},
            {"cell_type": "markdown", "source": "<!-- nor this"},
            {"cell_type": "code", "source": "x", "outputs": [
              {"output_type": "stream", "text": "10%\r50%\r100%\ndone\r\n"}
            ]}
        ]}"#;
        let markdown = to_markdown(json).unwrap();
        assert!(markdown.starts_with("```python\nnot closed\n```\n\n<!-- nor this\n-->\n\n"), "{markdown}");
        // A comment's start inside code, or in a sentence, opens nothing.
        assert_eq!(closed("```html\n<!-- start\n```"), "```html\n<!-- start\n```");
        assert_eq!(closed("Use `<!--` to hide text."), "Use `<!--` to hide text.");
        assert!(markdown.ends_with("```output\n100%\ndone\n```\n\n"), "{markdown}");
        // As the preview reads it: the code cell is code, not part of the comment.
        let blocks = crate::markdown_view::parse(&markdown);
        assert_eq!(blocks.len(), 3, "{blocks:?}");
    }

    #[test]
    fn code_holding_backticks_keeps_its_fence() {
        let json = r#"{"metadata": {}, "cells": [{"cell_type": "code", "source": "s = '```'", "outputs": []}]}"#;
        let markdown = to_markdown(json).unwrap();
        assert_eq!(markdown, "````python\ns = '```'\n````\n\n");
        // As the preview reads it: Python, all of it.
        let blocks = crate::markdown_view::parse(&markdown);
        assert!(
            matches!(&blocks[..], [crate::markdown_view::Block::Code { language, text, .. }] if language == "python" && text == "s = '```'"),
            "{blocks:?}"
        );
        assert_eq!(to_markdown("{\"not\": 1}"), None);
        assert_eq!(to_markdown("not json"), None);
    }
}
