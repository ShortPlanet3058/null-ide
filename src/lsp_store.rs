use crate::lsp::{LanguageServer, ServerMessage, path_for, uri_for};
use futures::StreamExt;
use gpui::{Context, EventEmitter, Task};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument, Exit, Initialized,
};
use lsp_types::request::{
    Completion, Formatting, GotoDefinition, HoverRequest, Initialize, References, Rename, Shutdown,
};
use lsp_types::{
    ClientCapabilities, CompletionClientCapabilities, CompletionContext, CompletionItem, CompletionItemCapability,
    CompletionParams, CompletionResponse, CompletionTriggerKind, Diagnostic, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverClientCapabilities, HoverParams, InitializeParams, InitializedParams,
    MarkupKind, Position, PublishDiagnosticsClientCapabilities, PublishDiagnosticsParams,
    TextDocumentClientCapabilities, TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, VersionedTextDocumentIdentifier, WindowClientCapabilities, WorkspaceFolder,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

type ServerConfig = crate::servers::Server;

fn config_for(path: &Path) -> Option<&'static ServerConfig> {
    crate::servers::for_path(path)
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
    /// Not installed.
    Missing,
    Installing,
    InstallFailed(String),
    /// Installed, but it failed to start.
    Unavailable,
}

/// Whether code intelligence is usable for a file yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Readiness {
    /// The server isn't installed: Null can install it, or says what's needed.
    Missing {
        server: &'static ServerConfig,
    },
    Installing {
        server: &'static ServerConfig,
    },
    InstallFailed {
        server: &'static ServerConfig,
    },
    /// Installed, but it failed to start.
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
    /// A server finished installing (its name), or failed to (why).
    Installed(Result<&'static str, String>),
}

/// The language servers for one project, and what they've reported.
pub struct LspStore {
    root: PathBuf,
    servers: HashMap<&'static str, ServerState>,
    /// Open files and their text, to hand to a server installed after they were opened.
    documents: HashMap<PathBuf, String>,
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
            documents: HashMap::new(),
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
            Some(ServerState::Missing) => Readiness::Missing { server: config },
            Some(ServerState::Installing) => Readiness::Installing { server: config },
            Some(ServerState::InstallFailed(_)) => Readiness::InstallFailed { server: config },
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

    /// Whether a server is being installed, or why its last install failed.
    pub fn install_state(&self, name: &str) -> Option<Result<(), &str>> {
        match self.servers.get(name)? {
            ServerState::Installing => Some(Ok(())),
            ServerState::InstallFailed(message) => Some(Err(message)),
            _ => None,
        }
    }

    /// Installs the server for `path` in the background, then starts it for every open
    /// file it serves.
    pub fn install(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Some(config) = config_for(path) {
            self.install_server(config, cx);
        }
    }

    pub fn install_server(&mut self, config: &'static ServerConfig, cx: &mut Context<Self>) {
        if matches!(self.servers.get(config.name), Some(ServerState::Installing | ServerState::Running { .. })) {
            return;
        }
        self.servers.insert(config.name, ServerState::Installing);
        cx.notify();
        self._tasks.push(cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { crate::servers::install(config) }).await;
            this.update(cx, |this, cx| {
                cx.emit(LspEvent::Installed(result.clone().map(|()| config.name)));
                match result {
                    Ok(()) => {
                        this.servers.remove(config.name);
                        let open: Vec<(PathBuf, String)> = this
                            .documents
                            .iter()
                            .filter(|(p, _)| config_for(p).is_some_and(|c| c.name == config.name))
                            .map(|(p, t)| (p.clone(), t.clone()))
                            .collect();
                        for (path, text) in open {
                            this.open(&path, text, cx);
                        }
                    }
                    Err(message) => {
                        this.servers.insert(config.name, ServerState::InstallFailed(message));
                    }
                }
                cx.emit(LspEvent::DiagnosticsChanged);
                cx.notify();
            })
            .ok();
        }));
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
        let Some(program) = crate::servers::find(config) else { return ServerState::Missing };
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
                    rename: Some(lsp_types::RenameClientCapabilities::default()),
                    references: Some(Default::default()),
                    formatting: Some(Default::default()),
                    completion: Some(CompletionClientCapabilities {
                        // Plain text only: Null doesn't do snippet placeholders yet.
                        completion_item: Some(CompletionItemCapability {
                            snippet_support: Some(false),
                            label_details_support: Some(true),
                            ..Default::default()
                        }),
                        context_support: Some(true),
                        ..Default::default()
                    }),
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
        let (Some(uri), Some(_)) = (uri_for(path), config_for(path)) else { return };
        self.documents.insert(path.to_path_buf(), text.clone());
        let language_id = crate::servers::language_id(path);
        self.with_server(
            path,
            move |server| {
                server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
                    text_document: TextDocumentItem { uri, language_id: language_id.into(), version: 0, text },
                })
            },
            cx,
        );
    }

    /// Sends the whole new text. Simple, and fast enough for files people edit by hand.
    pub fn change(&mut self, path: &Path, text: String, version: i32, cx: &mut Context<Self>) {
        let Some(uri) = uri_for(path) else { return };
        if let Some(document) = self.documents.get_mut(path) {
            *document = text.clone();
        }
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
        self.documents.remove(path);
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

    /// Suggestions at `position`. `trigger` is the character just typed if it was
    /// one the server asked for (like `.`), otherwise the request counts as invoked.
    pub fn completion(
        &self,
        path: &Path,
        position: Position,
        trigger: Option<char>,
    ) -> impl Future<Output = Vec<CompletionItem>> + use<> {
        let request = self.server_for(path).zip(Self::position_params(path, position)).map(|(server, params)| {
            server.request::<Completion>(CompletionParams {
                text_document_position: params,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: Some(CompletionContext {
                    trigger_kind: if trigger.is_some() {
                        CompletionTriggerKind::TRIGGER_CHARACTER
                    } else {
                        CompletionTriggerKind::INVOKED
                    },
                    trigger_character: trigger.map(String::from),
                }),
            })
        });
        async move {
            let Some(request) = request else { return Vec::new() };
            match request.await.ok().flatten() {
                Some(CompletionResponse::Array(items)) => items,
                Some(CompletionResponse::List(list)) => list.items,
                None => Vec::new(),
            }
        }
    }

    /// Every place the symbol at `position` is used, its declaration included.
    pub fn references(
        &self,
        path: &Path,
        position: Position,
    ) -> impl Future<Output = Vec<lsp_types::Location>> + use<> {
        let request = self.server_for(path).zip(Self::position_params(path, position)).map(|(server, params)| {
            server.request::<References>(lsp_types::ReferenceParams {
                text_document_position: params,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: lsp_types::ReferenceContext { include_declaration: true },
            })
        });
        async move {
            let Some(request) = request else { return Vec::new() };
            request.await.ok().flatten().unwrap_or_default()
        }
    }

    /// The edits renaming the symbol at `position` to `new_name` takes, in every file.
    pub fn rename(
        &self,
        path: &Path,
        position: Position,
        new_name: String,
    ) -> impl Future<Output = Result<lsp_types::WorkspaceEdit, String>> + use<> {
        let request = self.server_for(path).zip(Self::position_params(path, position)).map(|(server, params)| {
            server.request::<Rename>(lsp_types::RenameParams {
                text_document_position: params,
                new_name,
                work_done_progress_params: Default::default(),
            })
        });
        async move {
            let Some(request) = request else { return Err("No language server for this file.".into()) };
            match request.await {
                Ok(Some(edit)) => Ok(edit),
                Ok(None) => Err("This can't be renamed.".into()),
                Err(err) => Err(err.to_string()),
            }
        }
    }

    /// The edits that format the whole file.
    pub fn format(&self, path: &Path, tab_size: u32) -> impl Future<Output = Vec<lsp_types::TextEdit>> + use<> {
        let request = self.server_for(path).zip(uri_for(path)).map(|(server, uri)| {
            server.request::<Formatting>(lsp_types::DocumentFormattingParams {
                text_document: TextDocumentIdentifier { uri },
                options: lsp_types::FormattingOptions {
                    tab_size,
                    insert_spaces: true,
                    trim_trailing_whitespace: Some(true),
                    insert_final_newline: Some(true),
                    trim_final_newlines: Some(true),
                    ..Default::default()
                },
                work_done_progress_params: Default::default(),
            })
        });
        async move {
            let Some(request) = request else { return Vec::new() };
            request.await.ok().flatten().unwrap_or_default()
        }
    }

    /// Every problem the servers have reported, by file.
    pub fn all_diagnostics(&self) -> impl Iterator<Item = (&PathBuf, &Diagnostic)> {
        self.diagnostics.iter().flat_map(|(path, list)| list.iter().map(move |d| (path, d)))
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
