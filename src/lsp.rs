//! A minimal Language Server Protocol client: JSON-RPC over a child process's stdio.

use futures::channel::{mpsc, oneshot};
use lsp_types::notification::Notification;
use lsp_types::request::Request;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

/// Something the server sent that isn't a reply to one of our requests.
pub enum ServerMessage {
    Notification { method: String, params: Value },
    Request { id: Value, method: String, params: Value },
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

/// With `NULL_LSP_LOG=/some/file` set, every message to and from language
/// servers is appended there. For figuring out why a server misbehaves.
fn trace(direction: &str, message: &str) {
    static LOG: std::sync::OnceLock<Option<Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let log = LOG.get_or_init(|| {
        let path = std::env::var_os("NULL_LSP_LOG")?;
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(Mutex::new)
    });
    if let Some(file) = log {
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let short: String = message.chars().take(600).collect();
        let _ = writeln!(file.lock().unwrap(), "{:.3} {direction} {short}", time.as_secs_f64());
    }
}

pub struct LanguageServer {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    next_id: AtomicI64,
    pending: Pending,
}

impl LanguageServer {
    /// Starts `program` in `root` and returns it with a stream of its messages.
    pub fn spawn(
        program: &Path,
        args: &[&str],
        root: &Path,
    ) -> io::Result<(Self, mpsc::UnboundedReceiver<ServerMessage>)> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let pending: Pending = Arc::default();
        let (tx, rx) = mpsc::unbounded();

        let replies = pending.clone();
        std::thread::Builder::new().name("lsp-reader".into()).spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(message) = read_message(&mut reader) {
                let id = message.get("id").cloned();
                let method = message.get("method").and_then(Value::as_str).map(str::to_owned);
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                match (id, method) {
                    (Some(id), None) => {
                        let reply = match message.get("error") {
                            Some(error) => {
                                Err(error.get("message").and_then(Value::as_str).unwrap_or("error").to_owned())
                            }
                            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        if let Some(waiter) = id.as_i64().and_then(|id| replies.lock().unwrap().remove(&id)) {
                            waiter.send(reply).ok();
                        }
                    }
                    (Some(id), Some(method)) => {
                        tx.unbounded_send(ServerMessage::Request { id, method, params }).ok();
                    }
                    (None, Some(method)) => {
                        tx.unbounded_send(ServerMessage::Notification { method, params }).ok();
                    }
                    (None, None) => {}
                }
            }
            // The server exited: fail whatever is still waiting.
            replies.lock().unwrap().clear();
        })?;

        let server = Self { stdin: Mutex::new(stdin), child: Mutex::new(child), next_id: AtomicI64::new(1), pending };
        Ok((server, rx))
    }

    fn send(&self, message: Value) {
        let body = message.to_string();
        trace("->", &body);
        let mut stdin = self.stdin.lock().unwrap();
        let _ = write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).and_then(|()| stdin.flush());
    }

    /// Sends a request; the returned future resolves with the server's reply.
    pub fn request<R: Request>(&self, params: R::Params) -> impl Future<Output = Result<R::Result, String>> + use<R> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": R::METHOD, "params": params }));
        async move {
            let value = rx.await.map_err(|_| "the language server stopped".to_string())??;
            serde_json::from_value(value).map_err(|e| e.to_string())
        }
    }

    pub fn notify<N: Notification>(&self, params: N::Params) {
        self.send(json!({ "jsonrpc": "2.0", "method": N::METHOD, "params": params }));
    }

    pub fn respond(&self, id: Value, result: Value) {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }
}

impl Drop for LanguageServer {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

/// Reads one `Content-Length`-framed JSON message. None at end of stream.
fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    loop {
        let mut length = None;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).ok()? == 0 {
                return None;
            }
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some(value) = header.strip_prefix("Content-Length:") {
                length = value.trim().parse::<usize>().ok();
            }
        }
        let Some(length) = length else { continue };
        let mut body = vec![0; length];
        reader.read_exact(&mut body).ok()?;
        if let Ok(value) = serde_json::from_slice::<Value>(&body) {
            trace("<-", &value.to_string());
            return Some(value);
        }
    }
}

pub fn uri_for(path: &Path) -> Option<lsp_types::Uri> {
    url::Url::from_file_path(path).ok()?.as_str().parse().ok()
}

pub fn path_for(uri: &lsp_types::Uri) -> Option<PathBuf> {
    url::Url::parse(uri.as_str()).ok()?.to_file_path().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_framed_messages() {
        let body = r#"{"jsonrpc":"2.0","method":"x"}"#;
        let input = format!("Content-Length: {}\r\nContent-Type: utf-8\r\n\r\n{body}", body.len());
        let mut reader = io::Cursor::new(input.into_bytes());
        assert_eq!(read_message(&mut reader).unwrap()["method"], "x");
        assert!(read_message(&mut reader).is_none());
    }

    #[test]
    fn converts_paths_and_uris() {
        let path = std::env::temp_dir().join("a b.rs");
        let uri = uri_for(&path).unwrap();
        assert!(uri.as_str().contains("a%20b.rs"));
        assert_eq!(path_for(&uri).unwrap(), path);
    }
}

/// Runs only on request (`cargo test -- --ignored`): needs rust-analyzer installed.
#[cfg(test)]
mod rust_analyzer_tests {
    use super::*;
    use futures::StreamExt;
    use lsp_types::notification::{DidOpenTextDocument, Initialized};
    use lsp_types::request::{HoverRequest, Initialize};
    use lsp_types::*;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore]
    fn reports_problems_and_hovers() {
        let dir = std::env::temp_dir().join(format!("null-ra-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
            .unwrap();
        let source = "mod missing;\n\n/// Says hi.\nfn greet() {}\n\nfn main() {\n    greet();\n}\n";
        std::fs::write(dir.join("src/main.rs"), source).unwrap();
        let file = dir.join("src/main.rs");

        let program = which("rust-analyzer").expect("rust-analyzer on PATH");
        let (server, mut messages) = LanguageServer::spawn(&program, &[], &dir).unwrap();
        futures::executor::block_on(async {
            #[allow(deprecated)]
            let init = InitializeParams { root_uri: uri_for(&dir), ..Default::default() };
            server.request::<Initialize>(init).await.unwrap();
            server.notify::<Initialized>(InitializedParams {});
            server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri_for(&file).unwrap(),
                    language_id: "rust".into(),
                    version: 0,
                    text: source.into(),
                },
            });

            // Answer what the server asks, and wait for a diagnostic about `mod missing`.
            let deadline = Instant::now() + Duration::from_secs(120);
            let mut found = false;
            while !found && Instant::now() < deadline {
                match messages.next().await {
                    Some(ServerMessage::Request { id, .. }) => server.respond(id, Value::Null),
                    Some(ServerMessage::Notification { method, params })
                        if method == "textDocument/publishDiagnostics" =>
                    {
                        let params: PublishDiagnosticsParams = serde_json::from_value(params).unwrap();
                        found = params.diagnostics.iter().any(|d| d.range.start.line == 0);
                    }
                    Some(_) => {}
                    None => break,
                }
            }
            assert!(found, "expected a diagnostic on `mod missing;`");

            let hover = server
                .request::<HoverRequest>(HoverParams {
                    text_document_position_params: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier { uri: uri_for(&file).unwrap() },
                        position: Position { line: 6, character: 5 },
                    },
                    work_done_progress_params: Default::default(),
                })
                .await
                .unwrap()
                .expect("hover over greet()");
            let text = format!("{:?}", hover.contents);
            assert!(text.contains("fn greet") && text.contains("Says hi"), "{text}");
        });
        std::fs::remove_dir_all(&dir).ok();
    }

    fn which(program: &str) -> Option<PathBuf> {
        std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(program)).find(|p| p.is_file())
    }
}
