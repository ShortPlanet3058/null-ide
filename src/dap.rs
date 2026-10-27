//! A minimal Debug Adapter Protocol client: requests, responses and events over a
//! debug adapter's stdio (lldb-dap), framed the way language servers are.

use futures::channel::{mpsc, oneshot};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
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
    /// An adapter on a server: the paths in what goes to it and comes from it, translated.
    server: Option<String>,
}

/// The adapter for a program in `cwd`: on its server for a folder on one (its path there).
pub fn find_adapter_for(cwd: &Path) -> Option<PathBuf> {
    let Some(remote) = crate::remote::of(cwd) else { return find_adapter() };
    let names: Vec<String> = std::iter::once("lldb-dap".to_string())
        .chain((14..=22).rev().map(|v| format!("lldb-dap-{v}")))
        .chain(std::iter::once("lldb-vscode".to_string()))
        .collect();
    let script = format!(
        "PATH=\"{}\"; for p in {}; do command -v \"$p\" && exit 0; done; exit 1",
        crate::remote::path_inside(),
        names.join(" ")
    );
    let found = crate::remote::run(&remote.host, &script).ok().filter(|o| o.success())?;
    let path = String::from_utf8_lossy(&found.stdout).lines().next()?.trim().to_string();
    path.starts_with('/').then(|| PathBuf::from(path))
}

/// What goes to an adapter on `host`: every path of Null's there (`/ssh:host:/…`) as the
/// server has it.
fn to_server(host: &str, value: Value) -> Value {
    match value {
        Value::String(text) => match crate::remote::of(Path::new(&text)).filter(|r| r.host == host) {
            Some(remote) => Value::String(remote.path),
            None => Value::String(text),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(|v| to_server(host, v)).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, to_server(host, v))).collect()),
        other => other,
    }
}

/// What comes from an adapter on `host`: the paths it names (a stack frame's file, a
/// breakpoint's) as Null reaches them.
fn for_null(host: &str, value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(|v| for_null(host, v)).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, v)| match v {
                    Value::String(text)
                        if matches!(key.as_str(), "path" | "program" | "cwd") && text.starts_with('/') =>
                    {
                        (key, Value::String(crate::remote::path(host, &text).to_string_lossy().into_owned()))
                    }
                    v => (key, for_null(host, v)),
                })
                .collect(),
        ),
        other => other,
    }
}

/// The debugger Null drives: lldb-dap, on the PATH or (on macOS) the one with Xcode; on
/// Linux also as distributions name it after LLVM's version (lldb-dap-18).
pub fn find_adapter() -> Option<PathBuf> {
    crate::tools::find("lldb-dap")
        // (The Mac's /usr/bin one only asks to install Xcode's tools.)
        .filter(|p| !cfg!(target_os = "macos") || !p.starts_with("/usr/bin"))
        .or_else(|| crate::servers::xcrun_find("lldb-dap"))
        .or_else(|| crate::tools::find("lldb-vscode"))
        .or_else(|| (14..=22).rev().find_map(|v| crate::tools::find(&format!("lldb-dap-{v}"))))
        .or_else(|| (14..=19).rev().find_map(|v| crate::tools::find(&format!("lldb-vscode-{v}"))))
}

/// Where to get lldb-dap, as each platform has it.
pub const GET_ADAPTER: &str = if cfg!(target_os = "macos") {
    "Debugging needs lldb-dap, which comes with Xcode. It wasn't found."
} else if cfg!(windows) {
    "Debugging needs lldb-dap, which comes with LLVM (llvm.org, or “winget install LLVM.LLVM”). It wasn't found."
} else {
    "Debugging needs lldb-dap, which comes with LLDB (your distribution's lldb package). It wasn't found."
};

impl DebugAdapter {
    /// Starts the adapter and returns it with a stream of its events and requests.
    pub fn spawn(program: &Path, cwd: &Path) -> io::Result<(Self, mpsc::UnboundedReceiver<AdapterMessage>)> {
        // A folder on a server: the adapter runs there, in the folder.
        let server = crate::remote::of(cwd);
        let mut command = match &server {
            Some(remote) => crate::remote::command(&remote.host, Some(&remote.path), &program.to_string_lossy(), &[]),
            None => {
                let mut command = crate::tools::command(program);
                command.current_dir(cwd);
                command
            }
        };
        let server = server.map(|r| r.host);
        let host = server.clone();
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let pending: Pending = Arc::default();
        let (tx, rx) = mpsc::unbounded();
        let replies = pending.clone();
        std::thread::Builder::new().name("dap-reader".into()).spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(message) = crate::lsp::read_message(&mut reader) {
                let message = match &host {
                    Some(host) => for_null(host, message),
                    None => message,
                };
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
        let adapter =
            Self { stdin: Mutex::new(stdin), child: Mutex::new(child), next_seq: AtomicI64::new(1), pending, server };
        Ok((adapter, rx))
    }

    fn send(&self, message: Value) {
        let message = match &self.server {
            Some(host) => to_server(host, message),
            None => message,
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_adapter_on_a_server_gets_its_paths_and_null_gets_its_own() {
        let file = crate::remote::path("box", "/home/me/app/src/main.rs").to_string_lossy().into_owned();
        let sent = to_server("box", json!({ "source": { "path": file }, "lines": [3], "name": "main.rs" }));
        assert_eq!(sent, json!({ "source": { "path": "/home/me/app/src/main.rs" }, "lines": [3], "name": "main.rs" }));
        let back = for_null(
            "box",
            json!({ "stackFrames": [{ "source": { "path": "/home/me/app/src/main.rs" }, "name": "/x" }] }),
        );
        assert_eq!(back["stackFrames"][0]["source"]["path"], file);
        assert_eq!(back["stackFrames"][0]["name"], "/x");
    }
}
