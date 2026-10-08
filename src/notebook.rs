//! Jupyter notebooks (`.ipynb`) as they read: their JSON turned into Markdown for the
//! preview (⌘⇧V). Text cells as written, code in its language, and what each cell printed
//! below it (an error's message without the terminal's colours, a chart as its picture).

use serde_json::Value;

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
    let metadata = &notebook["metadata"];
    let language = metadata["kernelspec"]["language"]
        .as_str()
        .or(metadata["language_info"]["name"].as_str())
        .unwrap_or("python")
        .to_lowercase();
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
