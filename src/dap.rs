//! A minimal Debug Adapter Protocol client: requests, responses and events over a
//! debug adapter's stdio (lldb-dap), framed the way language servers are.

use futures::channel::{mpsc, oneshot};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

/// Something the adapter sent that isn't a reply to one of our requests.
pub enum AdapterMessage {
    Event {
        event: String,
        body: Value,
    },
    /// The adapter asking us something (like running the program in a terminal).
    Request {
        seq: i64,
        command: String,
    },
}

pub struct DebugAdapter {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    next_seq: AtomicI64,
    pending: Pending,
}

/// The debugger Null drives: lldb-dap, on the PATH or (on macOS) the one with Xcode.
pub fn find_adapter() -> Option<PathBuf> {
    crate::tools::find("lldb-dap")
        .filter(|p| !p.starts_with("/usr/bin"))
        .or_else(|| crate::servers::xcrun_find("lldb-dap"))
        .or_else(|| crate::tools::find("lldb-vscode"))
}

impl DebugAdapter {
    /// Starts the adapter and returns it with a stream of its events and requests.
    pub fn spawn(program: &Path, cwd: &Path) -> io::Result<(Self, mpsc::UnboundedReceiver<AdapterMessage>)> {
        let mut child = Command::new(program)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let pending: Pending = Arc::default();
        let (tx, rx) = mpsc::unbounded();
        let replies = pending.clone();
        std::thread::Builder::new().name("dap-reader".into()).spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(message) = crate::lsp::read_message(&mut reader) {
                match message["type"].as_str() {
                    Some("response") => {
                        let reply = if message["success"].as_bool() == Some(true) {
                            Ok(message.get("body").cloned().unwrap_or(Value::Null))
                        } else {
                            let text = message["message"].as_str().or(message["body"]["error"]["format"].as_str());
                            Err(text.unwrap_or("the debugger refused").to_string())
                        };
                        let seq = message["request_seq"].as_i64();
                        if let Some(waiter) = seq.and_then(|s| replies.lock().unwrap().remove(&s)) {
                            waiter.send(reply).ok();
                        }
                    }
                    Some("event") => {
                        let event = message["event"].as_str().unwrap_or_default().to_string();
                        let body = message.get("body").cloned().unwrap_or(Value::Null);
                        tx.unbounded_send(AdapterMessage::Event { event, body }).ok();
                    }
                    Some("request") => {
                        let seq = message["seq"].as_i64().unwrap_or_default();
                        let command = message["command"].as_str().unwrap_or_default().to_string();
                        tx.unbounded_send(AdapterMessage::Request { seq, command }).ok();
                    }
                    _ => {}
                }
            }
            // The adapter exited: fail whatever is still waiting.
            replies.lock().unwrap().clear();
        })?;
        let adapter = Self { stdin: Mutex::new(stdin), child: Mutex::new(child), next_seq: AtomicI64::new(1), pending };
        Ok((adapter, rx))
    }

    fn send(&self, message: Value) {
        let body = message.to_string();
        let mut stdin = self.stdin.lock().unwrap();
        let _ = write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).and_then(|()| stdin.flush());
    }

    /// Sends a request; the future resolves with the response's body, or why it failed.
    pub fn request(&self, command: &str, arguments: Value) -> impl Future<Output = Result<Value, String>> + use<> {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(seq, tx);
        self.send(json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments }));
        async move { rx.await.map_err(|_| "the debugger stopped".to_string())? }
    }

    /// Answers a request the adapter made.
    pub fn respond(&self, request_seq: i64, command: &str, success: bool, body: Value) {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        self.send(json!({
            "seq": seq, "type": "response", "request_seq": request_seq,
            "command": command, "success": success, "body": body,
        }));
    }
}

impl Drop for DebugAdapter {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap().kill();
    }
}
