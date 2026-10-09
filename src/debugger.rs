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

/// Why a watched expression has no value, without the debugger's own marks:
/// `error: <user expression 4>:1:6: no member named 'len'` says `no member named 'len'`.
fn why_not(error: &str) -> String {
    let first = error.lines().next().unwrap_or_default().trim();
    let first = first.strip_prefix("error:").unwrap_or(first).trim_start();
    let message = match first.find(">:") {
        Some(at) if first.starts_with('<') => {
            first[at + 2..].trim_start_matches(|c: char| c.is_ascii_digit() || c == ':' || c == ' ')
        }
        _ => first,
    };
    if message.is_empty() { "not known here".to_string() } else { message.to_string() }
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

/// For a Rust program: the lldb command that teaches it Rust's types (so a `&str` or a
/// `String` shows its text), from the toolchain's own formatters, when they're there.
pub fn rust_formatters() -> Option<String> {
    let rustc = crate::tools::find("rustc")?;
    let output = std::process::Command::new(rustc).args(["--print", "sysroot"]).output().ok()?;
    let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let script = sysroot.join("lib/rustlib/etc/lldb_lookup.py");
    script.is_file().then(|| format!("command script import \"{}\"", script.display()))
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

/// An expression watched while paused, and what it came to at the last stop.
#[derive(Clone, Debug, PartialEq)]
pub struct Watch {
    pub expression: String,
    /// Its value, or why it has none (not in scope here); None until worked out.
    pub value: Option<Result<String, String>>,
}

pub struct Debugger {
    pub state: DebugState,
    /// Expressions watched, from one stop to the next (and one run to the next).
    pub watches: Vec<Watch>,
    adapter: Option<Arc<DebugAdapter>>,
    /// What the program printed, and what the debugger said.
    pub output: String,
    /// Looking at where it stopped (its calls, its variables).
    stop_task: Option<Task<()>>,
    /// Working out the watches.
    watch_task: Option<Task<()>>,
    /// The program being debugged, for the panel.
    pub program: Option<PathBuf>,
    /// Stop where the program fails: a Rust panic, a thrown exception.
    pub stop_on_errors: bool,
    /// What the adapter can stop on when thrown (lldb-dap: cpp_throw, objc_throw…).
    throw_filters: Vec<String>,
    _tasks: Vec<Task<()>>,
}

impl Default for Debugger {
    fn default() -> Self {
        Self {
            state: DebugState::Idle,
            watches: Vec::new(),
            adapter: None,
            output: String::new(),
            stop_task: None,
            watch_task: None,
            program: None,
            stop_on_errors: false,
            throw_filters: Vec::new(),
            _tasks: Vec::new(),
        }
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
        breakpoints: Vec<(PathBuf, Vec<crate::editor::Breakpoint>)>,
        init_commands: Vec<String>,
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
                        if event == "initialized"
                            && let Some(tx) = initialized_tx.take()
                        {
                            tx.send(()).ok();
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
            let capabilities = match init.await {
                Ok(capabilities) => capabilities,
                Err(error) => {
                    this.update(cx, |this, cx| this.fail(&format!("The debugger didn't start: {error}"), cx)).ok();
                    return;
                }
            };
            // What it can stop on when thrown: the "throw" ones (not every catch).
            let filters: Vec<String> = capabilities["exceptionBreakpointFilters"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| f["filter"].as_str())
                .filter(|f| f.contains("throw"))
                .map(str::to_string)
                .collect();
            this.update(cx, |this, _| this.throw_filters = filters).ok();
            let cwd_text = cwd.display().to_string();
            let launch = adapter.request(
                "launch",
                json!({
                    "program": program.display().to_string(), "cwd": cwd_text, "args": [], "stopOnEntry": false,
                    "initCommands": init_commands,
                }),
            );
            if initialized_rx.await.is_err() {
                return;
            }
            for (path, lines) in breakpoints {
                adapter.request("setBreakpoints", Self::breakpoint_args(&path, &lines)).await.ok();
            }
            // As it is now (it may have been switched while starting).
            let (stop_on_errors, filters) =
                this.update(cx, |this, _| (this.stop_on_errors, this.throw_filters.clone())).unwrap_or_default();
            if stop_on_errors {
                for (command, arguments) in Self::error_stops(true, &filters) {
                    adapter.request(command, arguments).await.ok();
                }
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

    fn breakpoint_args(path: &Path, lines: &[crate::editor::Breakpoint]) -> Value {
        let breakpoints: Vec<Value> = lines
            .iter()
            .map(|b| {
                use crate::editor::BreakWhen;
                match b.condition.as_deref().map(BreakWhen::read) {
                    Some(BreakWhen::Condition("")) | None => json!({ "line": b.line + 1 }),
                    Some(BreakWhen::Condition(condition)) => json!({ "line": b.line + 1, "condition": condition }),
                    Some(BreakWhen::Hit(count)) => json!({ "line": b.line + 1, "hitCondition": count.to_string() }),
                    // Prints (with {x} worked out), doesn't stop.
                    Some(BreakWhen::Log(message)) => json!({ "line": b.line + 1, "logMessage": message }),
                }
            })
            .collect();
        json!({ "source": { "path": path.display().to_string() }, "breakpoints": breakpoints })
    }

    /// The requests that make the program stop where it fails (`on`), or no longer: a
    /// Rust panic (its `rust_panic`), and what the adapter can stop on when thrown.
    fn error_stops(on: bool, filters: &[String]) -> Vec<(&'static str, Value)> {
        let functions = if on { json!([{ "name": "rust_panic" }]) } else { json!([]) };
        let filters = if on { json!(filters) } else { json!([]) };
        vec![
            ("setFunctionBreakpoints", json!({ "breakpoints": functions })),
            ("setExceptionBreakpoints", json!({ "filters": filters })),
        ]
    }

    /// Stopping where the program fails, on or off (at once, when it's running).
    pub fn set_stop_on_errors(&mut self, on: bool, cx: &mut Context<Self>) {
        self.stop_on_errors = on;
        if let (Some(adapter), true) = (&self.adapter, self.is_active()) {
            for (command, arguments) in Self::error_stops(on, &self.throw_filters) {
                drop(adapter.request(command, arguments));
            }
        }
        cx.notify();
    }

    /// While debugging: the breakpoints of one file changed.
    pub fn set_breakpoints(&mut self, path: &Path, lines: &[crate::editor::Breakpoint]) {
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
        // The latest stop is the one that counts: one before it, still being looked at, goes.
        self.stop_task = Some(cx.spawn(async move |this, cx| {
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
                this.evaluate_watches(cx);
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
        self.stop_task = Some(cx.spawn(async move |this, cx| {
            let locals = Self::locals_of(&adapter, id).await;
            this.update(cx, |this, cx| {
                // Still the same stop (not a new one, with other calls, since).
                if let DebugState::Stopped(stop) = &mut this.state
                    && stop.frames.get(frame).is_some_and(|f| f.id == id)
                {
                    stop.frame = frame;
                    stop.place = stop.frames[frame].place.clone();
                    stop.locals = locals;
                    let stop = stop.clone();
                    cx.emit(DebuggerEvent::Stopped(stop));
                    this.evaluate_watches(cx);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Watches `expression`: its value now if paused, and at every stop after.
    pub fn add_watch(&mut self, expression: &str, cx: &mut Context<Self>) {
        let expression = expression.trim();
        if expression.is_empty() || self.watches.iter().any(|w| w.expression == expression) {
            return;
        }
        self.watches.push(Watch { expression: expression.to_string(), value: None });
        self.evaluate_watches(cx);
        cx.notify();
    }

    pub fn remove_watch(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.watches.len() {
            self.watches.remove(ix);
            cx.notify();
        }
    }

    /// Works out every watch in the call looked at, while paused.
    fn evaluate_watches(&mut self, cx: &mut Context<Self>) {
        let (Some(adapter), DebugState::Stopped(stop)) = (self.adapter.clone(), &self.state) else { return };
        let Some(frame) = stop.frames.get(stop.frame).map(|f| f.id) else { return };
        let expressions: Vec<String> = self.watches.iter().map(|w| w.expression.clone()).collect();
        if expressions.is_empty() {
            return;
        }
        // A newer stop works them all out again: the one before isn't needed.
        self.watch_task = Some(cx.spawn(async move |this, cx| {
            let mut values = Vec::new();
            for expression in &expressions {
                let asked = json!({ "expression": expression, "frameId": frame, "context": "watch" });
                let value = match adapter.request("evaluate", asked).await {
                    Ok(body) => {
                        let result = body["result"].as_str().unwrap_or_default();
                        Ok(clean_value(result).unwrap_or_else(|| result.to_string()))
                    }
                    Err(error) => Err(why_not(&error)),
                };
                values.push((expression.clone(), value));
            }
            this.update(cx, |this, cx| {
                for (expression, value) in values {
                    if let Some(watch) = this.watches.iter_mut().find(|w| w.expression == expression) {
                        watch.value = Some(value);
                    }
                }
                cx.notify();
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
    fn a_watch_says_why_it_has_no_value() {
        assert_eq!(
            why_not("error: <user expression 4>:1:6: no member named 'len' in 'str'"),
            "no member named 'len' in 'str'"
        );
        assert_eq!(why_not("error: use of undeclared identifier 'x'\nmore"), "use of undeclared identifier 'x'");
        assert_eq!(why_not("the debugger stopped"), "the debugger stopped");
        assert_eq!(why_not(""), "not known here");
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

    /// A count, a message printed without stopping, and stopping on errors: as the adapter
    /// reads them.
    #[test]
    fn breakpoints_are_sent_as_the_adapter_reads_them() {
        // A count, and a message printed without stopping.
        let args = Debugger::breakpoint_args(
            Path::new("/a.rs"),
            &[
                crate::editor::Breakpoint { line: 0, condition: Some("5".into()) },
                crate::editor::Breakpoint { line: 1, condition: Some("log x is {x}".into()) },
            ],
        );
        assert_eq!(args["breakpoints"][0], json!({ "line": 1, "hitCondition": "5" }));
        let plain = Debugger::breakpoint_args(
            Path::new("/a.rs"),
            &[crate::editor::Breakpoint { line: 0, condition: Some("log:".into()) }],
        );
        assert_eq!(plain["breakpoints"][0], json!({ "line": 1 }), "an empty message: a plain breakpoint");
        assert_eq!(args["breakpoints"][1], json!({ "line": 2, "logMessage": "x is {x}" }));
        // Stopping on errors: a Rust panic, and what's thrown; off, neither.
        let on = Debugger::error_stops(true, &["cpp_throw".into()]);
        assert_eq!(on[0].1, json!({ "breakpoints": [{ "name": "rust_panic" }] }));
        assert_eq!(on[1].1, json!({ "filters": ["cpp_throw"] }));
        assert_eq!(Debugger::error_stops(false, &["cpp_throw".into()])[1].1, json!({ "filters": [] }));
    }

    /// Needs lldb-dap and clang (they come with Xcode):
    /// `cargo test log_points_print_and_counts_stop -- --ignored`
    #[gpui::test]
    #[ignore]
    fn log_points_print_and_counts_stop(cx: &mut TestAppContext) {
        let dir = crate::tools::test_dir("debug-log");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("main.c");
        std::fs::write(
            &source,
            "#include <stdio.h>\nint main(void) {\n    int total = 0;\n    for (int i = 1; i <= 4; i++) {\n        total += i;\n    }\n    printf(\"total %d\\n\", total);\n    return 0;\n}\n",
        )
        .unwrap();
        let program = dir.join("main");
        let built = std::process::Command::new("xcrun").args(["clang", "-g", "-O0", "-o"]).arg(&program).arg(&source).status();
        assert!(built.unwrap().success());
        // A log point on `total += i;`: prints each time, never stops.
        let debugger = cx.new(|_| Debugger::default());
        let log = crate::editor::Breakpoint { line: 4, condition: Some("log i is {i}".into()) };
        debugger.update(cx, |d, cx| d.start(program.clone(), dir.clone(), vec![(source.clone(), vec![log])], Vec::new(), cx));
        assert!(wait_for(cx, &debugger, |d| d.state == DebugState::Idle), "stopped, or never ended");
        let output = debugger.read_with(cx, |d, _| d.output.clone());
        assert!(output.contains("i is 1") && output.contains("i is 4") && output.contains("total 10"), "{output}");
        // A count: stops the third time only (i is 3, total 1 + 2).
        let debugger = cx.new(|_| Debugger::default());
        let third = crate::editor::Breakpoint { line: 4, condition: Some("3".into()) };
        debugger.update(cx, |d, cx| d.start(program.clone(), dir.clone(), vec![(source.clone(), vec![third])], Vec::new(), cx));
        assert!(wait_for(cx, &debugger, |d| matches!(d.state, DebugState::Stopped(_))), "never stopped");
        let locals = debugger.read_with(cx, |d, _| match &d.state {
            DebugState::Stopped(stop) => stop.locals.clone(),
            _ => unreachable!(),
        });
        assert!(locals.iter().any(|(name, value)| name == "total" && value == "3"), "{locals:?}");
        // On from there: the count was reached, it stops every time after too (lldb's).
        debugger.update(cx, |d, cx| d.resume("continue", cx));
        let again = |d: &Debugger| {
            matches!(&d.state, DebugState::Stopped(s) if s.locals.iter().any(|(n, v)| n == "total" && v == "6"))
        };
        assert!(wait_for(cx, &debugger, again), "from the 3rd time on");
        debugger.update(cx, |d, cx| d.stop(cx));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Needs lldb-dap and clang (they come with Xcode):
    /// `cargo test stops_where_the_program_fails -- --ignored`
    #[gpui::test]
    #[ignore]
    fn stops_where_the_program_fails(cx: &mut TestAppContext) {
        let dir = crate::tools::test_dir("debug-throw");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("main.cpp");
        std::fs::write(&source, "#include <stdexcept>\nint main() {\n    throw std::runtime_error(\"no\");\n}\n").unwrap();
        let program = dir.join("main");
        let built = std::process::Command::new("xcrun").args(["clang++", "-g", "-O0", "-o"]).arg(&program).arg(&source).status();
        assert!(built.unwrap().success());
        let debugger = cx.new(|_| Debugger::default());
        debugger.update(cx, |d, cx| {
            d.set_stop_on_errors(true, cx);
            d.start(program.clone(), dir.clone(), Vec::new(), Vec::new(), cx)
        });
        assert!(wait_for(cx, &debugger, |d| matches!(d.state, DebugState::Stopped(_))), "never stopped");
        let reason = debugger.read_with(cx, |d, _| match &d.state {
            DebugState::Stopped(stop) => stop.reason.clone(),
            _ => unreachable!(),
        });
        assert!(reason == "exception" || reason == "breakpoint", "{reason}");
        debugger.update(cx, |d, cx| d.stop(cx));
        // A Rust panic.
        let source = dir.join("main.rs");
        std::fs::write(&source, "fn main() {\n    let v: Vec<u8> = Vec::new();\n    println!(\"{}\", v[3]);\n}\n").unwrap();
        let program = dir.join("panics");
        let built = std::process::Command::new("rustc").args(["-g", "-o"]).arg(&program).arg(&source).status();
        assert!(built.unwrap().success());
        let debugger = cx.new(|_| Debugger::default());
        debugger.update(cx, |d, cx| {
            d.set_stop_on_errors(true, cx);
            d.start(program.clone(), dir.clone(), Vec::new(), Vec::new(), cx)
        });
        assert!(wait_for(cx, &debugger, |d| matches!(d.state, DebugState::Stopped(_))), "never stopped at the panic");
        debugger.update(cx, |d, cx| d.stop(cx));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Needs lldb-dap and clang (they come with Xcode):
    /// `cargo test debugs_a_c_program -- --ignored`
    #[gpui::test]
    #[ignore]
    fn debugs_a_c_program(cx: &mut TestAppContext) {
        let dir = crate::tools::test_dir("debug");
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
        // A breakpoint on `total += i;` (line 5, from 0: 4), only once i is 3.
        let at = crate::editor::Breakpoint { line: 4, condition: Some("i == 3".into()) };
        debugger.update(cx, |d, cx| {
            d.start(program.clone(), dir.clone(), vec![(source.clone(), vec![at])], Vec::new(), cx)
        });
        assert!(wait_for(cx, &debugger, |d| matches!(d.state, DebugState::Stopped(_))), "never stopped");
        let stop = debugger.read_with(cx, |d, _| match &d.state {
            DebugState::Stopped(stop) => stop.clone(),
            _ => unreachable!(),
        });
        assert_eq!(stop.reason, "breakpoint");
        let (path, line) = stop.place.clone().unwrap();
        assert_eq!((path.file_name().unwrap().to_str().unwrap(), line), ("main.c", 4));
        // Its locals, with their values, and the call it's in.
        // Stopped only once i is 3: total is 1 + 2 by then.
        assert!(stop.locals.iter().any(|(name, value)| name == "total" && value == "3"), "{:?}", stop.locals);
        assert_eq!(stop.frames[0].name, "main");
        // Watched expressions: worked out here, or why not.
        debugger.update(cx, |d, cx| {
            d.add_watch("total * 2", cx);
            d.add_watch("nothing_called_this", cx);
            d.add_watch("total * 2", cx);
        });
        assert!(wait_for(cx, &debugger, |d| d.watches.iter().all(|w| w.value.is_some())), "never worked out");
        let watches = debugger.read_with(cx, |d, _| d.watches.clone());
        assert_eq!(watches.len(), 2, "the same one once");
        assert_eq!(watches[0].value, Some(Ok("6".to_string())));
        assert!(matches!(&watches[1].value, Some(Err(_))), "{:?}", watches[1]);
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
