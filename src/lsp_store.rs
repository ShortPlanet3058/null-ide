use crate::lsp::{LanguageServer, ServerMessage, path_for, uri_for};
use futures::StreamExt;
use gpui::{Context, EventEmitter, Task};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument, Exit, Initialized,
};
use lsp_types::request::{GotoDefinition, HoverRequest, Initialize, Shutdown};
use lsp_types::{
    ClientCapabilities, Diagnostic, DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverClientCapabilities,
    HoverParams, InitializeParams, InitializedParams, MarkupKind, Position, PublishDiagnosticsClientCapabilities,
    PublishDiagnosticsParams, TextDocumentClientCapabilities, TextDocumentContentChangeEvent, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, VersionedTextDocumentIdentifier, WindowClientCapabilities,
    WorkspaceFolder,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// How to start the server for one language.
struct ServerConfig {
    name: &'static str,
    /// The language, as people call it.
    label: &'static str,
    language_id: &'static str,
    program: &'static str,
    args: &'static [&'static str],
}

const SERVERS: &[(&[&str], ServerConfig)] = &[
    (
        &["rs"],
        ServerConfig { name: "rust-analyzer", label: "Rust", language_id: "rust", program: "rust-analyzer", args: &[] },
    ),
    (
        &["ts", "tsx", "js", "jsx", "mjs", "cjs"],
        ServerConfig {
            name: "typescript-language-server",
            label: "TypeScript",
            language_id: "typescript",
            program: "typescript-language-server",
            args: &["--stdio"],
        },
    ),
    (
        &["py"],
        ServerConfig {
            name: "pyright",
            label: "Python",
            language_id: "python",
            program: "pyright-langserver",
            args: &["--stdio"],
        },
    ),
    (&["go"], ServerConfig { name: "gopls", label: "Go", language_id: "go", program: "gopls", args: &[] }),
];

fn config_for(path: &Path) -> Option<&'static ServerConfig> {
    let ext = path.extension()?.to_str()?;
    SERVERS.iter().find(|(exts, _)| exts.contains(&ext)).map(|(_, config)| config)
}

/// Finds `program` on PATH, or in the usual install folders when Null was
/// started from the Finder with a minimal PATH.
fn find_program(program: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let extra = [
        home.as_ref().map(|h| h.join(".cargo/bin")),
        home.as_ref().map(|h| h.join("go/bin")),
        Some(PathBuf::from("/opt/homebrew/bin")),
        Some(PathBuf::from("/usr/local/bin")),
    ];
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(extra.into_iter().flatten())
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// Work for a server that's still starting up.
type Queued = Vec<Box<dyn FnOnce(&LanguageServer)>>;

enum ServerState {
    /// Waiting for the reply to `initialize`. Messages are queued until then.
    Starting {
        server: Arc<LanguageServer>,
        queued: Queued,
    },
    Running {
        server: Arc<LanguageServer>,
    },
    Unavailable,
}

/// Whether code intelligence is usable for a file yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Readiness {
    /// The server isn't installed or failed to start.
    Unavailable {
        program: &'static str,
    },
    Starting,
    /// Reading the project. Hover and go to definition don't answer yet.
    Indexing {
        percent: Option<u64>,
    },
    /// Ready. `checking` while the server compiles to find errors.
    Ready {
        checking: bool,
    },
}

struct Progress {
    server: &'static str,
    title: String,
    percent: Option<u64>,
}

pub enum LspEvent {
    DiagnosticsChanged,
}

/// The language servers for one project, and what they've reported.
pub struct LspStore {
    root: PathBuf,
    servers: HashMap<&'static str, ServerState>,
    diagnostics: HashMap<PathBuf, Vec<Diagnostic>>,
    /// Work servers report, by progress token.
    progress: HashMap<String, Progress>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<LspEvent> for LspStore {}

impl LspStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            servers: HashMap::new(),
            diagnostics: HashMap::new(),
            progress: HashMap::new(),
            _tasks: Vec::new(),
        }
    }

    pub fn diagnostics(&self, path: &Path) -> &[Diagnostic] {
        self.diagnostics.get(path).map_or(&[], Vec::as_slice)
    }

    pub fn language_label(path: &Path) -> Option<&'static str> {
        config_for(path).map(|c| c.label)
    }

    pub fn readiness(&self, path: &Path) -> Option<Readiness> {
        let config = config_for(path)?;
        Some(match self.servers.get(config.name) {
            None | Some(ServerState::Starting { .. }) => Readiness::Starting,
            Some(ServerState::Unavailable) => Readiness::Unavailable { program: config.program },
            Some(ServerState::Running { .. }) => {
                let work: Vec<&Progress> = self.progress.values().filter(|p| p.server == config.name).collect();
                let is_check = |p: &&Progress| p.title.to_lowercase().contains("check");
                match work.iter().find(|p| !is_check(p)) {
                    Some(indexing) => Readiness::Indexing { percent: indexing.percent },
                    None => Readiness::Ready { checking: work.iter().any(is_check) },
                }
            }
        })
    }

    pub fn has_server_for(&self, path: &Path) -> bool {
        config_for(path).is_some()
    }

    /// Runs `f` with the server for `path`, starting it if needed. Before the
    /// server has initialized, `f` is queued.
    fn with_server(&mut self, path: &Path, f: impl FnOnce(&LanguageServer) + 'static, cx: &mut Context<Self>) {
        let Some(config) = config_for(path) else { return };
        if !self.servers.contains_key(config.name) {
            let state = self.start(config, cx);
            self.servers.insert(config.name, state);
        }
        match self.servers.get_mut(config.name) {
            Some(ServerState::Running { server }) => f(server),
            Some(ServerState::Starting { queued, .. }) => queued.push(Box::new(f)),
            _ => {}
        }
    }

    fn server_for(&self, path: &Path) -> Option<Arc<LanguageServer>> {
        match self.servers.get(config_for(path)?.name)? {
            ServerState::Running { server } => Some(server.clone()),
            _ => None,
        }
    }

    fn start(&mut self, config: &'static ServerConfig, cx: &mut Context<Self>) -> ServerState {
        let Some(program) = find_program(config.program) else {
            eprintln!("null: {} isn't installed, so there's no code intelligence for it", config.name);
            return ServerState::Unavailable;
        };
        let (server, mut messages) = match LanguageServer::spawn(&program, config.args, &self.root) {
            Ok(started) => started,
            Err(err) => {
                eprintln!("null: couldn't start {}: {err}", config.name);
                return ServerState::Unavailable;
            }
        };
        let server = Arc::new(server);

        self._tasks.push(cx.spawn(async move |this, cx| {
            while let Some(message) = messages.next().await {
                if this.update(cx, |this, cx| this.handle_message(config, message, cx)).is_err() {
                    break;
                }
            }
        }));

        let root_uri = uri_for(&self.root);
        #[allow(deprecated)] // root_uri is deprecated in favour of workspace folders, but some servers still read it
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: root_uri.clone(),
            workspace_folders: root_uri.map(|uri| {
                vec![WorkspaceFolder {
                    uri,
                    name: self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                }]
            }),
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    hover: Some(HoverClientCapabilities {
                        content_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                        ..Default::default()
                    }),
                    publish_diagnostics: Some(PublishDiagnosticsClientCapabilities::default()),
                    ..Default::default()
                }),
                window: Some(WindowClientCapabilities { work_done_progress: Some(true), ..Default::default() }),
                ..Default::default()
            },
            ..Default::default()
        };
        let initialize = server.request::<Initialize>(params);
        self._tasks.push(cx.spawn(async move |this, cx| {
            let result = initialize.await;
            this.update(cx, |this, _| {
                let Some(ServerState::Starting { server, queued }) = this.servers.remove(config.name) else { return };
                match result {
                    Ok(_) => {
                        server.notify::<Initialized>(InitializedParams {});
                        for f in queued {
                            f(&server);
                        }
                        this.servers.insert(config.name, ServerState::Running { server });
                    }
                    Err(err) => {
                        eprintln!("null: {} failed to start: {err}", config.name);
                        this.servers.insert(config.name, ServerState::Unavailable);
                    }
                }
            })
            .ok();
        }));
        ServerState::Starting { server, queued: Vec::new() }
    }

    fn handle_message(&mut self, config: &'static ServerConfig, message: ServerMessage, cx: &mut Context<Self>) {
        match message {
            ServerMessage::Notification { method, params } => match method.as_str() {
                "textDocument/publishDiagnostics" => {
                    let Ok(params) = serde_json::from_value::<PublishDiagnosticsParams>(params) else { return };
                    let Some(path) = path_for(&params.uri) else { return };
                    self.diagnostics.insert(path, params.diagnostics);
                    cx.emit(LspEvent::DiagnosticsChanged);
                    cx.notify();
                }
                "$/progress" => {
                    let token = params["token"].to_string();
                    let value = &params["value"];
                    match value["kind"].as_str() {
                        Some("begin") => {
                            let title = value["title"].as_str().unwrap_or_default().to_string();
                            let percent = value["percentage"].as_u64();
                            self.progress.insert(token, Progress { server: config.name, title, percent });
                        }
                        Some("report") => {
                            if let Some(progress) = self.progress.get_mut(&token) {
                                progress.percent = value["percentage"].as_u64().or(progress.percent);
                            }
                        }
                        _ => {
                            self.progress.remove(&token);
                        }
                    }
                    cx.notify();
                }
                _ => {}
            },
            ServerMessage::Request { id, method, params } => {
                // Answer what servers ask during startup so they don't wait on us.
                let result = match method.as_str() {
                    "workspace/configuration" => {
                        let count = params["items"].as_array().map_or(0, Vec::len);
                        Value::Array(vec![Value::Null; count])
                    }
                    _ => Value::Null,
                };
                if let Some(server) = match self.servers.get(config.name) {
                    Some(ServerState::Running { server } | ServerState::Starting { server, .. }) => Some(server),
                    _ => None,
                } {
                    server.respond(id, result);
                }
            }
        }
    }

    pub fn open(&mut self, path: &Path, text: String, cx: &mut Context<Self>) {
        let (Some(uri), Some(config)) = (uri_for(path), config_for(path)) else { return };
        self.with_server(
            path,
            move |server| {
                server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
                    text_document: TextDocumentItem { uri, language_id: config.language_id.into(), version: 0, text },
                })
            },
            cx,
        );
    }

    /// Sends the whole new text. Simple, and fast enough for files people edit by hand.
    pub fn change(&mut self, path: &Path, text: String, version: i32, cx: &mut Context<Self>) {
        let Some(uri) = uri_for(path) else { return };
        self.with_server(
            path,
            move |server| {
                server.notify::<DidChangeTextDocument>(DidChangeTextDocumentParams {
                    text_document: VersionedTextDocumentIdentifier { uri, version },
                    content_changes: vec![TextDocumentContentChangeEvent { range: None, range_length: None, text }],
                })
            },
            cx,
        );
    }

    pub fn save(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(uri) = uri_for(path) else { return };
        self.with_server(
            path,
            move |server| {
                server.notify::<DidSaveTextDocument>(DidSaveTextDocumentParams {
                    text_document: TextDocumentIdentifier { uri },
                    text: None,
                })
            },
            cx,
        );
    }

    pub fn close(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(uri) = uri_for(path) else { return };
        self.with_server(
            path,
            move |server| {
                server.notify::<DidCloseTextDocument>(DidCloseTextDocumentParams {
                    text_document: TextDocumentIdentifier { uri },
                })
            },
            cx,
        );
    }

    fn position_params(path: &Path, position: Position) -> Option<TextDocumentPositionParams> {
        Some(TextDocumentPositionParams { text_document: TextDocumentIdentifier { uri: uri_for(path)? }, position })
    }

    pub fn hover(&self, path: &Path, position: Position) -> impl Future<Output = Option<Hover>> + use<> {
        let request = self.server_for(path).zip(Self::position_params(path, position)).map(|(server, params)| {
            server.request::<HoverRequest>(HoverParams {
                text_document_position_params: params,
                work_done_progress_params: Default::default(),
            })
        });
        async move { request?.await.ok().flatten() }
    }

    pub fn definition(
        &self,
        path: &Path,
        position: Position,
    ) -> impl Future<Output = Vec<lsp_types::Location>> + use<> {
        let request = self.server_for(path).zip(Self::position_params(path, position)).map(|(server, params)| {
            server.request::<GotoDefinition>(GotoDefinitionParams {
                text_document_position_params: params,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
        });
        async move {
            let Some(request) = request else { return Vec::new() };
            match request.await.ok().flatten() {
                Some(GotoDefinitionResponse::Scalar(location)) => vec![location],
                Some(GotoDefinitionResponse::Array(locations)) => locations,
                Some(GotoDefinitionResponse::Link(links)) => links
                    .into_iter()
                    .map(|l| lsp_types::Location { uri: l.target_uri, range: l.target_selection_range })
                    .collect(),
                None => Vec::new(),
            }
        }
    }

    /// Stops every server politely.
    pub fn shutdown(&mut self) {
        for state in self.servers.values() {
            if let ServerState::Running { server } = state {
                drop(server.request::<Shutdown>(()));
                server.notify::<Exit>(());
            }
        }
        self.servers.clear();
    }
}
