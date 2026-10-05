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
    /// The file and line (from 0) of the top frame, when it's in source code.
    pub place: Option<(PathBuf, usize)>,
    /// "breakpoint", "step", "pause", "exception"…
    pub reason: String,
    /// What the debugger said about it (an exception's message).
    pub description: Option<String>,
}

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

    /// Asks where thread `thread` stopped, then says so.
    fn locate_stop(&mut self, thread: i64, reason: String, description: Option<String>, cx: &mut Context<Self>) {
        let Some(adapter) = self.adapter.clone() else { return };
        let frames = adapter.request("stackTrace", json!({ "threadId": thread, "startFrame": 0, "levels": 1 }));
        self._tasks.push(cx.spawn(async move |this, cx| {
            let place = frames.await.ok().and_then(|body| {
                let frame = body["stackFrames"].get(0)?;
                let path = PathBuf::from(frame["source"]["path"].as_str()?);
                let line = frame["line"].as_u64()?.saturating_sub(1) as usize;
                Some((path, line))
            });
            this.update(cx, |this, cx| {
                let stop = Stop { thread, place, reason, description };
                this.state = DebugState::Stopped(stop.clone());
                cx.emit(DebuggerEvent::Stopped(stop));
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
        let (path, line) = stop.place.unwrap();
        assert_eq!((path.file_name().unwrap().to_str().unwrap(), line), ("main.c", 4));
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
