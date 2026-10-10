//! For looking at Null as it is while working on it (debug builds only): with
//! `NULL_QA=<file>`, the steps in that file are done in the window once it's open, one a
//! line, and `ready` says on stdout when to take the picture (`scripts/qa/shot.sh`, which
//! keeps it apart from the person's settings, files and environment).
//!
//! Steps: `open <path>`, `goto <line>:<column>` (from 1), `keys <keystroke> ...`
//! (`cmd-p`, `escape`), `type <text>`, `action <name>` (`workspace::ShowOutline`;
//! `terminal` for the terminal), `run <command>` (in the
//! terminal), `wait <ms>`, `rows` (the caret line's rows as drawn, with what's after its
//! text, to stderr), `shot <name>` (a picture now, `<name>.png` beside the last one),
//! `ready` (the last: steps after it aren't done). A line starting with `#` is a note.
//!
//! A run copies to and pastes from a clipboard of its own, never reads or changes the
//! keychain, and installs nothing (see `system_clipboard::kept_apart`).

use crate::workspace::Workspace;
use gpui::{App, AppContext as _, Keystroke, Modifiers};
use std::time::Duration;

pub fn run(cx: &mut App) {
    let Some(file) = std::env::var_os("NULL_QA") else { return };
    let steps = match std::fs::read_to_string(&file) {
        Ok(steps) => steps,
        Err(err) => return eprintln!("null qa: couldn't read the steps: {err}"),
    };
    cx.spawn(async move |cx| {
        // The window first: opened, drawn, its files read.
        cx.background_executor().timer(Duration::from_millis(800)).await;
        let ack = std::env::var_os("NULL_QA_ACK").map(std::path::PathBuf::from);
        let mut shots = 0;
        for line in steps.lines().map(str::trim_start).filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
            // (What `type` types, exactly as written after the space: spaces count.)
            let (step, arg) = line.split_once(' ').map_or((line.trim_end(), ""), |(s, a)| match s {
                "type" => (s, a),
                _ => (s, a.trim()),
            });
            eprintln!("null qa: {line}");
            if step == "shot" {
                // Drawn, then the script takes it while this waits for it to say it has
                // (a file in NULL_QA_ACK named for the shot's number), at most 15 s.
                cx.background_executor().timer(Duration::from_millis(300)).await;
                shots += 1;
                println!("NULL_QA_SHOT {arg}");
                let taken = ack.as_ref().map(|dir| dir.join(format!("{shots}.done")));
                for _ in 0..300 {
                    if taken.as_ref().is_none_or(|done| done.exists()) {
                        break;
                    }
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                }
                continue;
            }
            if step == "wait" {
                let ms = arg.parse().unwrap_or(500);
                cx.background_executor().timer(Duration::from_millis(ms)).await;
                continue;
            }
            let Ok(Some(handle)) = cx.update(|cx| cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>()))
            else {
                return eprintln!("null qa: no window");
            };
            let done = match step {
                // Keys go to the window, not to the workspace (it's free to take them).
                "keys" | "type" => cx.update_window(handle.into(), |_, window, cx| {
                    let keys: Vec<Keystroke> = if step == "keys" {
                        arg.split_whitespace().filter_map(|k| Keystroke::parse(k).ok()).collect()
                    } else {
                        arg.chars()
                            .map(|c| Keystroke {
                                modifiers: Modifiers::default(),
                                key: c.to_lowercase().to_string(),
                                key_char: Some(c.to_string()),
                            })
                            .collect()
                    };
                    for key in keys {
                        window.dispatch_keystroke(key, cx);
                    }
                }),
                "action" => cx.update_window(handle.into(), |_, window, cx| {
                    let name = if arg == "terminal" { "workspace::ToggleTerminal" } else { arg };
                    match cx.build_action(name, None) {
                        Ok(action) => window.dispatch_action(action, cx),
                        Err(err) => eprintln!("null qa: no action {name}: {err}"),
                    }
                }),
                "rows" => handle.update(cx, |workspace, _, cx| {
                    if let Some(editor) = workspace.qa_editor() {
                        let editor = editor.read(cx);
                        let line = editor.buffer.point(editor.selection.head).0;
                        for row in editor.layout.iter().flat_map(|l| &l.rows).filter(|r| r.row.line == line) {
                            eprintln!("null qa: row {:?}", row.shaped.text.to_string());
                        }
                    }
                }),
                "open" | "goto" | "run" => handle.update(cx, |workspace, window, cx| match step {
                    // (A path from the project's folder, as a file of it is written.)
                    "open" => {
                        let path = workspace.root(cx).join(arg);
                        workspace.open_file(path, window, cx)
                    }
                    "run" => workspace.qa_run(arg.to_string(), window, cx),
                    _ => {
                        let (line, column) = arg.split_once(':').unwrap_or((arg, "1"));
                        let place = (line.parse::<usize>().unwrap_or(1).max(1) - 1, column.parse::<usize>().unwrap_or(1).max(1) - 1);
                        if let Some(editor) = workspace.qa_editor() {
                            editor.update(cx, |e, cx| e.set_caret_point(place, cx));
                        }
                    }
                }),
                "ready" => {
                    println!("NULL_QA_READY");
                    break;
                }
                other => {
                    eprintln!("null qa: no step called {other}");
                    Ok(())
                }
            };
            if let Err(err) = done {
                eprintln!("null qa: {line}: {err}");
            }
            // Drawn before the next step.
            cx.background_executor().timer(Duration::from_millis(150)).await;
        }
    })
    .detach();
}
