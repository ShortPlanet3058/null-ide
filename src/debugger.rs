//! Debugging a program with lldb-dap: start it with the breakpoints set, see where it
//! stopped, step through it, and read what it printed.

use crate::dap::{AdapterMessage, DebugAdapter};
use futures::StreamExt;
use gpui::{Context, EventEmitter, Task};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Output kept for the Debug panel, at most (older lines go).
const MAX_OUTPUT: usize = 200_000;

#[derive(Clone, Debug, PartialEq)]
pub enum DebugState {
    Idle,
    /// Building the program first (`cargo build`).
    Building,
    Starting,
    Running,
    Stopped(Stop),
}

/// Where and why the program stopped.
#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub thread: i64,
    /// The file and line (from 0) of the frame looked at, when it's in source code.
    pub place: Option<(PathBuf, usize)>,
    /// "breakpoint", "step", "pause", "exception"…
    pub reason: String,
    /// What the debugger said about it (an exception's message).
    pub description: Option<String>,
    /// The calls that led here, innermost first.
    pub frames: Vec<Frame>,
    /// The frame looked at (0: where it stopped).
    pub frame: usize,
    /// That frame's local variables: name and value.
    pub locals: Vec<(String, String)>,
}

/// One call on the stack.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub id: i64,
    pub name: String,
    pub place: Option<(PathBuf, usize)>,
}

/// A value as worth showing: without the address the debugger adds (`… @ 0x7ff…`), and
/// None when what's left only names a type (`&str`, `Vec<u8>`): that says nothing new.
pub fn clean_value(value: &str) -> Option<String> {
    let value = match value.rfind(" @ 0x") {
        Some(at) if value[at + 5..].chars().all(|c| c.is_ascii_hexdigit()) => &value[..at],
        _ => value,
    }
    .trim();
    // Text or a structure's fields are data; a reference, a path or generics with neither
    // is a type (`&str[3]`, `IntoIter<&str>`), even with a number in it.
    let has_data = value.contains(['"', '\'', '{']) || matches!(value, "true" | "false");
    let type_like = value.is_empty() || value.starts_with('&') || value.contains("::") || value.contains('<');
    (has_data || !type_like).then(|| value.to_string())
}

/// Whether `name` appears in `text` as a whole word.
fn names(text: &str, name: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    !name.is_empty()
        && text.match_indices(name).any(|(i, _)| {
            let before = text[..i].chars().next_back();
            let after = text[i + name.len()..].chars().next();
            !before.is_some_and(is_word) && !after.is_some_and(is_word)
        })
}

/// The code of a line, without a `//` comment (it names things too, but isn't about them).
fn code_of(text: &str) -> &str {
    text.split("//").next().unwrap_or(text)
}

/// The locals the code shows (named in `lines`), with values worth showing: what the
/// Debug panel lists. Compiler temporaries (a loop's `iter`) aren't in the code.
pub fn shown_locals(lines: &[(usize, String)], locals: &[(String, String)]) -> Vec<(String, String)> {
    locals
        .iter()
        .filter(|(name, _)| lines.iter().any(|(_, text)| names(code_of(text), name)))
        .filter_map(|(name, value)| Some((name.clone(), clean_value(value)?)))
        .collect()
}

/// The values to show faintly at the end of lines: each local once, on the last of
/// `lines` that names it, as `name = value` (at most three a line, long ones cut short).
pub fn inline_values(lines: &[(usize, String)], locals: &[(String, String)]) -> Vec<(usize, String)> {
    const PER_LINE: usize = 3;
    const LONGEST: usize = 32;
    let mut by_line: Vec<(usize, Vec<String>)> = Vec::new();
    for (name, value) in shown_locals(lines, locals) {
        let Some((line, _)) = lines.iter().rev().find(|(_, text)| names(code_of(text), &name)) else { continue };
        let value: String = if value.chars().count() > LONGEST {
            value.chars().take(LONGEST - 1).chain(['…']).collect()
        } else {
            value
        };
        let shown = format!("{name} = {value}");
        match by_line.iter_mut().find(|(l, _)| l == line) {
            Some((_, list)) if list.len() < PER_LINE => list.push(shown),
            Some(_) => {}
            None => by_line.push((*line, vec![shown])),
        }
    }
    by_line.sort_by_key(|(line, _)| *line);
    by_line.into_iter().map(|(line, list)| (line, list.join("   "))).collect()
}

/// How many calls of the stack are asked for.
const FRAMES: usize = 40;

pub enum DebuggerEvent {
    /// Stopped somewhere: the workspace shows the place.
    Stopped(Stop),
    /// Running again, or finished: no place to show.
    Resumed,
    Ended,
}

impl EventEmitter<DebuggerEvent> for Debugger {}

pub struct Debugger {
    pub state: DebugState,
    adapter: Option<Arc<DebugAdapter>>,
    /// What the program printed, and what the debugger said.
    pub output: String,
    /// The program being debugged, for the panel.
    pub program: Option<PathBuf>,
    _tasks: Vec<Task<()>>,
}

impl Default for Debugger {
    fn default() -> Self {
        Self { state: DebugState::Idle, adapter: None, output: String::new(), program: None, _tasks: Vec::new() }
    }
}

impl Debugger {
    pub fn is_active(&self) -> bool {
        self.state != DebugState::Idle
    }

    pub fn append_output(&mut self, text: &str, cx: &mut Context<Self>) {
        self.output.push_str(text);
        if self.output.len() > MAX_OUTPUT {
            let cut = self.output.len() - MAX_OUTPUT;
            let cut = (cut..self.output.len()).find(|&i| self.output.is_char_boundary(i)).unwrap_or(cut);
            self.output.drain(..cut);
        }
        cx.notify();
    }

    pub fn set_building(&mut self, program: Option<PathBuf>, cx: &mut Context<Self>) {
        self.output.clear();
        self.program = program;
        self.state = DebugState::Building;
        cx.notify();
    }

    /// Runs `program` in `cwd` under the debugger, stopping at `breakpoints` (lines from 0).
    pub fn start(
        &mut self,
        program: PathBuf,
        cwd: PathBuf,
        breakpoints: Vec<(PathBuf, Vec<usize>)>,
        cx: &mut Context<Self>,
    ) {
        if self.state != DebugState::Building {
            self.output.clear();
        }
        self.program = Some(program.clone());
        let Some(adapter_path) = crate::dap::find_adapter() else {
            self.state = DebugState::Idle;
            let message = "Null debugs with lldb-dap, which comes with Xcode (or LLVM): it wasn't found.\n";
            return self.append_output(message, cx);
        };
        let (adapter, mut messages) = match DebugAdapter::spawn(&adapter_path, &cwd) {
            Ok(started) => started,
            Err(error) => {
                self.state = DebugState::Idle;
                return self.append_output(&format!("Couldn't start lldb-dap: {error}\n"), cx);
            }
        };
        let adapter = Arc::new(adapter);
        self.adapter = Some(adapter.clone());
        self.state = DebugState::Starting;
        cx.notify();

        // Its events: stops, output, the end.
        let (initialized_tx, initialized_rx) = futures::channel::oneshot::channel::<()>();
        let mut initialized_tx = Some(initialized_tx);
        let events = cx.spawn(async move |this, cx| {
            while let Some(message) = messages.next().await {
                let alive = this.update(cx, |this, cx| match message {
                    AdapterMessage::Event { event, body } => {
                        if event == "initialized" {
                            if let Some(tx) = initialized_tx.take() {
                                tx.send(()).ok();
                            }
                        }
                        this.handle_event(&event, body, cx);
                    }
                    // Null shows the output itself: decline to run the program in a terminal.
                    AdapterMessage::Request { seq, command, .. } => {
                        if let Some(adapter) = &this.adapter {
                            adapter.respond(seq, &command, false, Value::Null);
                        }
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
            // The adapter went away.
            this.update(cx, |this, cx| this.ended(cx)).ok();
        });

        // The handshake: initialize, launch, then the breakpoints once it's ready for them.
        let handshake = cx.spawn(async move |this, cx| {
            let init = adapter.request(
                "initialize",
                json!({
                    "clientID": "null", "clientName": "Null", "adapterID": "lldb-dap",
                    "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path",
                    "supportsRunInTerminalRequest": false,
                }),
            );
            if let Err(error) = init.await {
                this.update(cx, |this, cx| this.fail(&format!("The debugger didn't start: {error}"), cx)).ok();
                return;
            }
            let cwd_text = cwd.display().to_string();
            let launch = adapter.request(
                "launch",
                json!({ "program": program.display().to_string(), "cwd": cwd_text, "args": [], "stopOnEntry": false }),
            );
            if initialized_rx.await.is_err() {
                return;
            }
            for (path, lines) in breakpoints {
                adapter.request("setBreakpoints", Self::breakpoint_args(&path, &lines)).await.ok();
            }
            adapter.request("configurationDone", json!({})).await.ok();
            match launch.await {
                Ok(_) => {
                    this.update(cx, |this, cx| {
                        if this.state == DebugState::Starting {
                            this.state = DebugState::Running;
                            cx.notify();
                        }
                    })
                    .ok();
                }
                Err(error) => {
                    this.update(cx, |this, cx| this.fail(&format!("Couldn't run the program: {error}"), cx)).ok();
                }
            }
        });
        self._tasks = vec![events, handshake];
    }

    fn breakpoint_args(path: &Path, lines: &[usize]) -> Value {
        let breakpoints: Vec<Value> = lines.iter().map(|l| json!({ "line": l + 1 })).collect();
        json!({ "source": { "path": path.display().to_string() }, "breakpoints": breakpoints })
    }

    /// While debugging: the breakpoints of one file changed.
    pub fn set_breakpoints(&mut self, path: &Path, lines: &[usize]) {
        if let (Some(adapter), true) = (&self.adapter, self.is_active()) {
            drop(adapter.request("setBreakpoints", Self::breakpoint_args(path, lines)));
        }
    }

    fn handle_event(&mut self, event: &str, body: Value, cx: &mut Context<Self>) {
        match event {
            "output" => {
                let category = body["category"].as_str().unwrap_or("console");
                if category != "telemetry"
                    && let Some(text) = body["output"].as_str()
                {
                    self.append_output(text, cx);
                }
            }
            "stopped" => {
                let thread = body["threadId"].as_i64().unwrap_or(1);
                let reason = body["reason"].as_str().unwrap_or("pause").to_string();
                let description = body["text"].as_str().or(body["description"].as_str()).map(str::to_string);
                self.locate_stop(thread, reason, description, cx);
            }
            "continued" => {
                self.state = DebugState::Running;
                cx.emit(DebuggerEvent::Resumed);
                cx.notify();
            }
            "exited" => {
                let code = body["exitCode"].as_i64().unwrap_or_default();
                self.append_output(&format!("\nThe program ended (exit code {code}).\n"), cx);
            }
            "terminated" => self.ended(cx),
            _ => {}
        }
    }

    /// Asks where thread `thread` stopped (the calls, and the locals of the innermost one
    /// in source code), then says so.
    fn locate_stop(&mut self, thread: i64, reason: String, description: Option<String>, cx: &mut Context<Self>) {
        let Some(adapter) = self.adapter.clone() else { return };
        let stack = adapter.request("stackTrace", json!({ "threadId": thread, "startFrame": 0, "levels": FRAMES }));
        self._tasks.push(cx.spawn(async move |this, cx| {
            let frames: Vec<Frame> = stack
                .await
                .ok()
                .and_then(|body| body["stackFrames"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .map(|f| Frame {
                    id: f["id"].as_i64().unwrap_or_default(),
                    name: f["name"].as_str().unwrap_or("?").to_string(),
                    place: f["source"]["path"]
                        .as_str()
                        .map(|path| (PathBuf::from(path), f["line"].as_u64().unwrap_or(1).saturating_sub(1) as usize)),
                })
                .collect();
            // Stopped inside a library with no source: look at the first call that has some.
            let frame = frames.iter().position(|f| f.place.is_some()).unwrap_or(0);
            let locals = match frames.get(frame) {
                Some(f) => Self::locals_of(&adapter, f.id).await,
                None => Vec::new(),
            };
            this.update(cx, |this, cx| {
                let place = frames.get(frame).and_then(|f| f.place.clone());
                let stop = Stop { thread, place, reason, description, frames, frame, locals };
                this.state = DebugState::Stopped(stop.clone());
                cx.emit(DebuggerEvent::Stopped(stop));
                cx.notify();
            })
            .ok();
        }));
    }

    /// A frame's local variables, as the debugger shows them (name, value).
    async fn locals_of(adapter: &DebugAdapter, frame: i64) -> Vec<(String, String)> {
        let Ok(scopes) = adapter.request("scopes", json!({ "frameId": frame })).await else { return Vec::new() };
        // The first scope is the locals (then globals, registers: not wanted here).
        let Some(reference) = scopes["scopes"].get(0).and_then(|s| s["variablesReference"].as_i64()) else {
            return Vec::new();
        };
        let Ok(variables) = adapter.request("variables", json!({ "variablesReference": reference })).await else {
            return Vec::new();
        };
        variables["variables"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|v| Some((v["name"].as_str()?.to_string(), v["value"].as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Looks at another call on the stack: its place and its locals.
    pub fn select_frame(&mut self, frame: usize, cx: &mut Context<Self>) {
        let (Some(adapter), DebugState::Stopped(stop)) = (self.adapter.clone(), &self.state) else { return };
        let Some(id) = stop.frames.get(frame).map(|f| f.id) else { return };
        self._tasks.push(cx.spawn(async move |this, cx| {
            let locals = Self::locals_of(&adapter, id).await;
            this.update(cx, |this, cx| {
                if let DebugState::Stopped(stop) = &mut this.state {
                    stop.frame = frame;
                    stop.place = stop.frames[frame].place.clone();
                    stop.locals = locals;
                    let stop = stop.clone();
                    cx.emit(DebuggerEvent::Stopped(stop));
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn thread(&self) -> Option<i64> {
        match &self.state {
            DebugState::Stopped(stop) => Some(stop.thread),
            _ => None,
        }
    }

    /// Continue, step over, into or out: from where it stopped.
    pub fn resume(&mut self, command: &str, cx: &mut Context<Self>) {
        let (Some(adapter), Some(thread)) = (&self.adapter, self.thread()) else { return };
        drop(adapter.request(command, json!({ "threadId": thread })));
        self.state = DebugState::Running;
        cx.emit(DebuggerEvent::Resumed);
        cx.notify();
    }

    /// Stops a running program where it is.
    pub fn pause(&mut self) {
        if let (Some(adapter), DebugState::Running) = (&self.adapter, &self.state) {
            drop(adapter.request("pause", json!({ "threadId": 1 })));
        }
    }

    /// Ends the session, and the program with it.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(adapter) = &self.adapter {
            drop(adapter.request("disconnect", json!({ "terminateDebuggee": true })));
        }
        self.ended(cx);
    }

    fn fail(&mut self, message: &str, cx: &mut Context<Self>) {
        self.append_output(&format!("{message}\n"), cx);
        self.ended(cx);
    }

    fn ended(&mut self, cx: &mut Context<Self>) {
        if self.state == DebugState::Idle && self.adapter.is_none() {
            return;
        }
        self.adapter = None;
        self.state = DebugState::Idle;
        cx.emit(DebuggerEvent::Ended);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};

    #[test]
    fn values_show_on_the_lines_that_name_them() {
        let lines = vec![
            (2, "    int total = 0;".to_string()),
            (3, "    for (int i = 1; i <= 3; i++) { // total so far".to_string()),
            (4, "        total += i;".to_string()),
            (5, "    totals();".to_string()),
        ];
        let locals = vec![("total".to_string(), "3".to_string()), ("i".to_string(), "2".to_string())];
        // Each once, on the last line naming it; `totals` and the comment don't count.
        assert_eq!(inline_values(&lines, &locals), [(4, "total = 3   i = 2".to_string())]);
        let long = vec![("s".to_string(), "x".repeat(50))];
        assert_eq!(inline_values(&[(0, "s".into())], &long)[0].1.chars().count(), 4 + 32);
    }

    #[test]
    fn addresses_and_bare_types_are_left_out() {
        assert_eq!(clean_value("4"), Some("4".into()));
        assert_eq!(clean_value("\"is\" @ 0x7ff7bfefe558"), Some("\"is\"".into()));
        assert_eq!(clean_value("&str @ 0x7ff7bfefe558"), None);
        assert_eq!(clean_value("&str[3] @ 0x7ff7bfefe490"), None);
        assert_eq!(clean_value("-12.5"), Some("-12.5".into()));
        assert_eq!(clean_value("core::array::iter::IntoIter<&str> @ 0x7ff7bfefe508"), None);
        assert_eq!(clean_value("true"), Some("true".into()));
        // The loop's iterator isn't in the code: not listed.
        let lines = vec![(0, "for word in words {".to_string())];
        let locals = vec![("iter".to_string(), "x".to_string()), ("word".to_string(), "\"a\"".to_string())];
        assert_eq!(shown_locals(&lines, &locals), [("word".to_string(), "\"a\"".to_string())]);
    }

    /// Waits for real work (the adapter, the program) until `done` holds, or gives up.
    fn wait_for(cx: &mut TestAppContext, debugger: &gpui::Entity<Debugger>, done: impl Fn(&Debugger) -> bool) -> bool {
        for _ in 0..600 {
            cx.run_until_parked();
            if debugger.read_with(cx, |d, _| done(d)) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }

    /// Needs lldb-dap and clang (they come with Xcode):
    /// `cargo test debugs_a_c_program -- --ignored`
    #[gpui::test]
    #[ignore]
    fn debugs_a_c_program(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("null-debug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("main.c");
        std::fs::write(
            &source,
            "#include <stdio.h>\nint main(void) {\n    int total = 0;\n    for (int i = 1; i <= 3; i++) {\n        total += i;\n    }\n    printf(\"total %d\\n\", total);\n    return 0;\n}\n",
        )
        .unwrap();
        let program = dir.join("main");
        let built = std::process::Command::new("xcrun")
            .args(["clang", "-g", "-O0", "-o"])
            .arg(&program)
            .arg(&source)
            .status()
            .unwrap();
        assert!(built.success());
        let debugger = cx.new(|_| Debugger::default());
        // A breakpoint on `total += i;` (line 5, from 0: 4).
        debugger.update(cx, |d, cx| d.start(program.clone(), dir.clone(), vec![(source.clone(), vec![4])], cx));
        assert!(wait_for(cx, &debugger, |d| matches!(d.state, DebugState::Stopped(_))), "never stopped");
        let stop = debugger.read_with(cx, |d, _| match &d.state {
            DebugState::Stopped(stop) => stop.clone(),
            _ => unreachable!(),
        });
        assert_eq!(stop.reason, "breakpoint");
        let (path, line) = stop.place.clone().unwrap();
        assert_eq!((path.file_name().unwrap().to_str().unwrap(), line), ("main.c", 4));
        // Its locals, with their values, and the call it's in.
        assert!(stop.locals.iter().any(|(name, value)| name == "total" && value == "0"), "{:?}", stop.locals);
        assert_eq!(stop.frames[0].name, "main");
        // Step over: on to the loop's next line.
        debugger.update(cx, |d, cx| d.resume("next", cx));
        assert!(wait_for(cx, &debugger, |d| matches!(&d.state, DebugState::Stopped(s) if s.reason == "step")));
        // Forget the breakpoint and run to the end.
        debugger.update(cx, |d, cx| {
            d.set_breakpoints(&source, &[]);
            d.resume("continue", cx);
        });
        assert!(wait_for(cx, &debugger, |d| d.state == DebugState::Idle), "never ended");
        let output = debugger.read_with(cx, |d, _| d.output.clone());
        assert!(output.contains("total 6"), "{output}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
