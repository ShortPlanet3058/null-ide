use crate::lsp::{LanguageServer, ServerMessage, path_for, uri_for};
use futures::{FutureExt, StreamExt};
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
use ropey::Rope;
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

/// What a "go to" asks the server for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Definition,
    TypeDefinition,
    Implementation,
    /// Rust: the `mod` line that brings this file in (rust-analyzer).
    ParentModule,
    /// Rust: the crate's Cargo.toml (rust-analyzer).
    CargoToml,
}

pub enum LspEvent {
    DiagnosticsChanged,
    /// A server asked for edits to files, while running a quick fix's command.
    ApplyEdit(lsp_types::WorkspaceEdit),
    /// A server finished installing (its name), or failed to (why).
    Installed(Result<&'static str, String>),
}

/// rust-analyzer's own request for the web page documenting what's at a position.
pub enum ExternalDocs {}

/// Its answer: the page's address, or (when the client says it reads local docs) both.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum DocsLink {
    Web(String),
    Both { web: Option<String>, local: Option<String> },
}

impl DocsLink {
    pub fn web(self) -> Option<String> {
        match self {
            DocsLink::Web(url) => Some(url),
            DocsLink::Both { web, .. } => web,
        }
    }
}

impl lsp_types::request::Request for ExternalDocs {
    type Params = TextDocumentPositionParams;
    type Result = Option<DocsLink>;
    const METHOD: &'static str = "experimental/externalDocs";
}

/// rust-analyzer's own request for where a file's module is declared.
pub enum ParentModule {}

impl lsp_types::request::Request for ParentModule {
    type Params = TextDocumentPositionParams;
    type Result = Option<GotoDefinitionResponse>;
    const METHOD: &'static str = "experimental/parentModule";
}

/// rust-analyzer's own request for the crate's Cargo.toml.
pub enum OpenCargoToml {}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCargoTomlParams {
    pub text_document: TextDocumentIdentifier,
}

impl lsp_types::request::Request for OpenCargoToml {
    type Params = OpenCargoTomlParams;
    type Result = Option<lsp_types::Location>;
    const METHOD: &'static str = "experimental/openCargoToml";
}

/// rust-analyzer's own request for a macro's expansion.
pub enum ExpandMacro {}

impl lsp_types::request::Request for ExpandMacro {
    type Params = TextDocumentPositionParams;
    type Result = Option<ExpandedMacro>;
    const METHOD: &'static str = "rust-analyzer/expandMacro";
}

/// A macro's name and what it expands to.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExpandedMacro {
    pub name: String,
    pub expansion: String,
}

/// The language servers for one project, and what they've reported.
pub struct LspStore {
    root: PathBuf,
    servers: HashMap<&'static str, ServerState>,
    /// Open files and their text, to hand to a server installed after they were opened.
    documents: HashMap<PathBuf, Rope>,
    diagnostics: HashMap<PathBuf, Vec<Diagnostic>>,
    /// Goes up whenever any diagnostics change, so views can keep what they derived.
    diagnostics_version: u64,
    /// Goes up when a server says what names are has changed (it finished reading the
    /// project): files ask again.
    semantic_refresh: u64,
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
            diagnostics_version: 0,
            semantic_refresh: 0,
            progress: HashMap::new(),
            _tasks: Vec::new(),
        }
    }

    pub fn diagnostics_version(&self) -> u64 {
        self.diagnostics_version
    }

    pub fn semantic_refresh(&self) -> u64 {
        self.semantic_refresh
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
            Some(ServerState::Running { .. }) => match self.work_shown(config.name) {
                (Some(percent), _) => Readiness::Indexing { percent },
                (None, checking) => Readiness::Ready { checking },
            },
        })
    }

    /// What a running server's work shows as: indexing (and how far), or checking.
    fn work_shown(&self, server: &str) -> (Option<Option<u64>>, bool) {
        let work: Vec<&Progress> = self.progress.values().filter(|p| p.server == server).collect();
        let is_check = |p: &&Progress| p.title.to_lowercase().contains("check");
        (work.iter().find(|p| !is_check(p)).map(|p| p.percent), work.iter().any(is_check))
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
                        let open: Vec<(PathBuf, Rope)> = this
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

    /// Whether a language server answers for this file now.
    pub fn serves(&self, path: &Path) -> bool {
        self.server_for(path).is_some()
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
                    // Unused code is faded, deprecated names struck through.
                    publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                        tag_support: Some(lsp_types::TagSupport {
                            value_set: vec![
                                lsp_types::DiagnosticTag::UNNECESSARY,
                                lsp_types::DiagnosticTag::DEPRECATED,
                            ],
                        }),
                        ..Default::default()
                    }),
                    rename: Some(lsp_types::RenameClientCapabilities::default()),
                    references: Some(Default::default()),
                    type_definition: Some(Default::default()),
                    implementation: Some(Default::default()),
                    document_highlight: Some(Default::default()),
                    inlay_hint: Some(Default::default()),
                    semantic_tokens: Some(lsp_types::SemanticTokensClientCapabilities {
                        requests: lsp_types::SemanticTokensClientCapabilitiesRequests {
                            range: Some(false),
                            full: Some(lsp_types::SemanticTokensFullOptions::Bool(true)),
                        },
                        token_types: crate::editor::SEMANTIC_TYPES
                            .iter()
                            .map(|t| lsp_types::SemanticTokenType::new(t))
                            .collect(),
                        token_modifiers: ["declaration", "readonly", "static", "deprecated", "constant"]
                            .into_iter()
                            .map(lsp_types::SemanticTokenModifier::new)
                            .collect(),
                        formats: vec![lsp_types::TokenFormat::RELATIVE],
                        overlapping_token_support: Some(false),
                        multiline_token_support: Some(false),
                        // Its tokens add to the grammar's colours, they don't replace them.
                        augments_syntax_tokens: Some(true),
                        ..Default::default()
                    }),
                    signature_help: Some(lsp_types::SignatureHelpClientCapabilities {
                        signature_information: Some(lsp_types::SignatureInformationSettings {
                            documentation_format: Some(vec![MarkupKind::PlainText]),
                            parameter_information: Some(lsp_types::ParameterInformationSettings {
                                label_offset_support: Some(true),
                            }),
                            active_parameter_support: Some(true),
                        }),
                        ..Default::default()
                    }),
                    formatting: Some(Default::default()),
                    range_formatting: Some(Default::default()),
                    code_action: Some(lsp_types::CodeActionClientCapabilities {
                        code_action_literal_support: Some(lsp_types::CodeActionLiteralSupport {
                            code_action_kind: lsp_types::CodeActionKindLiteralSupport {
                                value_set: ["", "quickfix", "refactor", "refactor.extract", "refactor.inline"]
                                    .into_iter()
                                    .chain(["refactor.rewrite", "source", "source.organizeImports"])
                                    .map(String::from)
                                    .collect(),
                            },
                        }),
                        // Edits are worked out when one is picked, so asking stays quick.
                        resolve_support: Some(lsp_types::CodeActionCapabilityResolveSupport {
                            properties: vec!["edit".into()],
                        }),
                        data_support: Some(true),
                        is_preferred_support: Some(true),
                        disabled_support: Some(true),
                        ..Default::default()
                    }),
                    completion: Some(CompletionClientCapabilities {
                        // Placeholders to fill in with ⇥ (see editor::snippet).
                        completion_item: Some(CompletionItemCapability {
                            snippet_support: Some(true),
                            label_details_support: Some(true),
                            ..Default::default()
                        }),
                        context_support: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                window: Some(WindowClientCapabilities { work_done_progress: Some(true), ..Default::default() }),
                workspace: Some(lsp_types::WorkspaceClientCapabilities {
                    apply_edit: Some(true),
                    semantic_tokens: Some(lsp_types::SemanticTokensWorkspaceClientCapabilities {
                        refresh_support: Some(true),
                    }),
                    // Renaming a file can update the code that names it (`mod parser;`).
                    file_operations: Some(lsp_types::WorkspaceFileOperationsClientCapabilities {
                        will_rename: Some(true),
                        did_rename: Some(true),
                        ..Default::default()
                    }),
                    workspace_edit: Some(lsp_types::WorkspaceEditClientCapabilities {
                        document_changes: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
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
                    Ok(result) => {
                        server.set_capabilities(&result.capabilities);
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
                    self.diagnostics_version += 1;
                    cx.emit(LspEvent::DiagnosticsChanged);
                    cx.notify();
                }
                "$/progress" => {
                    // Servers report often (every file indexed): only a change to what's shown
                    // draws the window again.
                    let shown = self.work_shown(config.name);
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
                    if self.work_shown(config.name) != shown {
                        cx.notify();
                    }
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
                    "workspace/applyEdit" => {
                        let edit = serde_json::from_value::<lsp_types::ApplyWorkspaceEditParams>(params);
                        let applied = edit.is_ok();
                        if let Ok(edit) = edit {
                            cx.emit(LspEvent::ApplyEdit(edit.edit));
                        }
                        serde_json::json!({ "applied": applied })
                    }
                    "workspace/semanticTokens/refresh" => {
                        self.semantic_refresh += 1;
                        cx.notify();
                        Value::Null
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

    pub fn open(&mut self, path: &Path, text: Rope, cx: &mut Context<Self>) {
        let (Some(uri), Some(_)) = (uri_for(path), config_for(path)) else { return };
        self.documents.insert(path.to_path_buf(), text.clone());
        let language_id = crate::servers::language_id(path);
        self.with_server(
            path,
            move |server| {
                let text = text.to_string();
                server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
                    text_document: TextDocumentItem { uri, language_id: language_id.into(), version: 0, text },
                })
            },
            cx,
        );
    }

    /// Tells the server about a change: just the changed parts (`edits`) when it takes
    /// them, else the whole text. `edits` is None when they aren't known (after an undo).
    pub fn change(
        &mut self,
        path: &Path,
        text: Rope,
        edits: Option<Vec<TextDocumentContentChangeEvent>>,
        version: i32,
        cx: &mut Context<Self>,
    ) {
        let Some(uri) = uri_for(path) else { return };
        if let Some(document) = self.documents.get_mut(path) {
            *document = text.clone();
        }
        self.with_server(
            path,
            move |server| {
                let content_changes = match edits {
                    Some(edits) if server.incremental() => edits,
                    _ => {
                        vec![TextDocumentContentChangeEvent { range: None, range_length: None, text: text.to_string() }]
                    }
                };
                server.notify::<DidChangeTextDocument>(DidChangeTextDocumentParams {
                    text_document: VersionedTextDocumentIdentifier { uri, version },
                    content_changes,
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

    /// The web page documenting what's at `position`, from rust-analyzer (docs.rs, the
    /// standard library's docs).
    pub fn docs_link(&self, path: &Path, position: Position) -> impl Future<Output = Option<String>> + use<> {
        let request = self
            .server_for(path)
            .zip(Self::position_params(path, position))
            .map(|(server, params)| server.request::<ExternalDocs>(params));
        async move { request?.await.ok().flatten().and_then(DocsLink::web) }
    }

    /// What the macro at `position` expands to, from rust-analyzer: its name and the code.
    pub fn expand_macro(&self, path: &Path, position: Position) -> impl Future<Output = Option<ExpandedMacro>> + use<> {
        let request = self
            .server_for(path)
            .zip(Self::position_params(path, position))
            .map(|(server, params)| server.request::<ExpandMacro>(params));
        async move { request?.await.ok().flatten() }
    }

    fn position_params(path: &Path, position: Position) -> Option<TextDocumentPositionParams> {
        Some(TextDocumentPositionParams { text_document: TextDocumentIdentifier { uri: uri_for(path)? }, position })
    }

    /// The signature of the call around `position`, with the parameter it's at.
    pub fn signature_help(
        &self,
        path: &Path,
        position: Position,
    ) -> impl Future<Output = Option<lsp_types::SignatureHelp>> + use<> {
        let request = self.server_for(path).zip(Self::position_params(path, position)).map(|(server, params)| {
            server.request::<lsp_types::request::SignatureHelpRequest>(lsp_types::SignatureHelpParams {
                context: None,
                text_document_position_params: params,
                work_done_progress_params: Default::default(),
            })
        });
        async move { request?.await.ok().flatten() }
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
        self.locations_of(Target::Definition, path, position)
    }

    /// Every place the function at `position` is called from: the call itself, in its caller.
    pub fn callers(&self, path: &Path, position: Position) -> impl Future<Output = Vec<lsp_types::Location>> + use<> {
        let asked = self.server_for(path).zip(Self::position_params(path, position));
        async move {
            match asked {
                Some((server, params)) => crate::lsp::callers(&server, params).await,
                None => Vec::new(),
            }
        }
    }

    /// Where the symbol at `position` is defined, its type is, or it's implemented.
    pub fn locations_of(
        &self,
        target: Target,
        path: &Path,
        position: Position,
    ) -> impl Future<Output = Vec<lsp_types::Location>> + use<> {
        let request = self.server_for(path).zip(Self::position_params(path, position)).map(|(server, params)| {
            let params = GotoDefinitionParams {
                text_document_position_params: params,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            match target {
                Target::Definition => server.request::<GotoDefinition>(params).boxed_local(),
                Target::TypeDefinition => {
                    server.request::<lsp_types::request::GotoTypeDefinition>(params).boxed_local()
                }
                Target::Implementation => {
                    server.request::<lsp_types::request::GotoImplementation>(params).boxed_local()
                }
                Target::ParentModule => {
                    server.request::<ParentModule>(params.text_document_position_params).boxed_local()
                }
                Target::CargoToml => {
                    let params =
                        OpenCargoTomlParams { text_document: params.text_document_position_params.text_document };
                    let request = server.request::<OpenCargoToml>(params);
                    async move { request.await.map(|found| found.map(GotoDefinitionResponse::Scalar)) }.boxed_local()
                }
            }
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

    /// What each name in a file is (a type, a function…), as the server sees it: its
    /// tokens, and what their numbers stand for. None when it can't say.
    pub fn semantic_tokens(
        &self,
        path: &Path,
    ) -> impl Future<Output = Option<(Vec<lsp_types::SemanticToken>, Arc<crate::lsp::SemanticLegend>)>> + use<> {
        let request = self.server_for(path).zip(uri_for(path)).and_then(|(server, uri)| {
            let legend = server.semantic_legend()?;
            let request =
                server.request::<lsp_types::request::SemanticTokensFullRequest>(lsp_types::SemanticTokensParams {
                    text_document: TextDocumentIdentifier { uri },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                });
            Some((request, legend))
        });
        async move {
            let (request, legend) = request?;
            match request.await.ok().flatten()? {
                lsp_types::SemanticTokensResult::Tokens(tokens) => Some((tokens.data, legend)),
                lsp_types::SemanticTokensResult::Partial(partial) => Some((partial.data, legend)),
            }
        }
    }

    /// The type and parameter hints for `range` of a file.
    pub fn inlay_hints(
        &self,
        path: &Path,
        range: lsp_types::Range,
    ) -> impl Future<Output = Vec<lsp_types::InlayHint>> + use<> {
        let request = self.server_for(path).zip(uri_for(path)).map(|(server, uri)| {
            server.request::<lsp_types::request::InlayHintRequest>(lsp_types::InlayHintParams {
                text_document: TextDocumentIdentifier { uri },
                range,
                work_done_progress_params: Default::default(),
            })
        });
        async move {
            let Some(request) = request else { return Vec::new() };
            request.await.ok().flatten().unwrap_or_default()
        }
    }

    /// The servers that care about renaming `path`: its language's, or for a folder,
    /// every one running (a folder can hold a module).
    fn servers_for_rename(&self, path: &Path) -> Vec<Arc<LanguageServer>> {
        if let Some(server) = self.server_for(path) {
            return vec![server];
        }
        if path.is_file() {
            return Vec::new();
        }
        self.servers
            .values()
            .filter_map(|state| match state {
                ServerState::Running { server } => Some(server.clone()),
                _ => None,
            })
            .collect()
    }

    fn rename_params(from: &Path, to: &Path) -> Option<lsp_types::RenameFilesParams> {
        let (old, new) = (uri_for(from)?, uri_for(to)?);
        Some(lsp_types::RenameFilesParams {
            files: vec![lsp_types::FileRename { old_uri: old.to_string(), new_uri: new.to_string() }],
        })
    }

    /// The edits to make before renaming `from` to `to`, so the code naming it still does.
    pub fn will_rename(&self, from: &Path, to: &Path) -> impl Future<Output = Vec<lsp_types::WorkspaceEdit>> + use<> {
        let requests: Vec<_> = match Self::rename_params(from, to) {
            Some(params) => self
                .servers_for_rename(from)
                .into_iter()
                .map(|server| server.request::<lsp_types::request::WillRenameFiles>(params.clone()))
                .collect(),
            None => Vec::new(),
        };
        async move {
            let mut edits = Vec::new();
            for request in requests {
                if let Ok(Some(edit)) = request.await {
                    edits.push(edit);
                }
            }
            edits
        }
    }

    /// Tells the servers a file or folder was renamed.
    pub fn did_rename(&self, from: &Path, to: &Path) {
        let Some(params) = Self::rename_params(from, to) else { return };
        // Renamed already: the servers are found from the new path.
        for server in self.servers_for_rename(to) {
            server.notify::<lsp_types::notification::DidRenameFiles>(params.clone());
        }
    }

    /// Where the symbol at `position` is used in this file.
    pub fn document_highlight(
        &self,
        path: &Path,
        position: Position,
    ) -> impl Future<Output = Vec<lsp_types::DocumentHighlight>> + use<> {
        let request = self.server_for(path).zip(Self::position_params(path, position)).map(|(server, params)| {
            server.request::<lsp_types::request::DocumentHighlightRequest>(lsp_types::DocumentHighlightParams {
                text_document_position_params: params,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
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
    pub fn format(
        &self,
        path: &Path,
        tab_size: u32,
        insert_spaces: bool,
    ) -> impl Future<Output = Vec<lsp_types::TextEdit>> + use<> {
        self.format_part(path, None, tab_size, insert_spaces)
    }

    /// Whether the server for `path` can format part of a file.
    pub fn formats_ranges(&self, path: &Path) -> bool {
        self.server_for(path).is_some_and(|s| s.formats_ranges())
    }

    /// The edits that format `range` of the file (the whole of it without one).
    pub fn format_part(
        &self,
        path: &Path,
        range: Option<lsp_types::Range>,
        tab_size: u32,
        insert_spaces: bool,
    ) -> impl Future<Output = Vec<lsp_types::TextEdit>> + use<> {
        let options = lsp_types::FormattingOptions {
            tab_size,
            insert_spaces,
            trim_trailing_whitespace: Some(true),
            insert_final_newline: Some(true),
            trim_final_newlines: Some(true),
            ..Default::default()
        };
        let request = self.server_for(path).zip(uri_for(path)).map(|(server, uri)| {
            let text_document = TextDocumentIdentifier { uri };
            match range {
                None => server
                    .request::<Formatting>(lsp_types::DocumentFormattingParams {
                        text_document,
                        options,
                        work_done_progress_params: Default::default(),
                    })
                    .boxed_local(),
                Some(range) => server
                    .request::<lsp_types::request::RangeFormatting>(lsp_types::DocumentRangeFormattingParams {
                        text_document,
                        range,
                        options,
                        work_done_progress_params: Default::default(),
                    })
                    .boxed_local(),
            }
        });
        async move {
            let Some(request) = request else { return Vec::new() };
            request.await.ok().flatten().unwrap_or_default()
        }
    }

    /// The fixes and refactorings the server offers for `range`, given the problems there.
    pub fn code_actions(
        &self,
        path: &Path,
        range: lsp_types::Range,
        diagnostics: Vec<Diagnostic>,
    ) -> impl Future<Output = Vec<lsp_types::CodeActionOrCommand>> + use<> {
        self.code_actions_of(path, range, diagnostics, None)
    }

    /// The code actions of the kinds in `only` (all when None): `source.organizeImports`…
    pub fn code_actions_of(
        &self,
        path: &Path,
        range: lsp_types::Range,
        diagnostics: Vec<Diagnostic>,
        only: Option<Vec<lsp_types::CodeActionKind>>,
    ) -> impl Future<Output = Vec<lsp_types::CodeActionOrCommand>> + use<> {
        let request = self.server_for(path).zip(uri_for(path)).map(|(server, uri)| {
            server.request::<lsp_types::request::CodeActionRequest>(lsp_types::CodeActionParams {
                text_document: TextDocumentIdentifier { uri },
                range,
                context: lsp_types::CodeActionContext {
                    diagnostics,
                    only,
                    trigger_kind: Some(lsp_types::CodeActionTriggerKind::INVOKED),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
        });
        async move {
            let Some(request) = request else { return Vec::new() };
            request.await.ok().flatten().unwrap_or_default()
        }
    }

    /// Fills in the edit of an action that came without one.
    pub fn resolve_code_action(
        &self,
        path: &Path,
        action: lsp_types::CodeAction,
    ) -> impl Future<Output = Result<lsp_types::CodeAction, String>> + use<> {
        let request =
            self.server_for(path).map(|server| server.request::<lsp_types::request::CodeActionResolveRequest>(action));
        async move {
            match request {
                Some(request) => request.await,
                None => Err("No language server for this file.".into()),
            }
        }
    }

    /// The rest of a completion the server left out of the list (the import it adds, for
    /// typescript-language-server), asked for once it's picked.
    pub fn resolve_completion(
        &self,
        path: &Path,
        item: lsp_types::CompletionItem,
    ) -> impl Future<Output = Result<lsp_types::CompletionItem, String>> + use<> {
        let request =
            self.server_for(path).map(|server| server.request::<lsp_types::request::ResolveCompletionItem>(item));
        async move {
            match request {
                Some(request) => request.await,
                None => Err("No language server for this file.".into()),
            }
        }
    }

    /// Runs a server command (a fix that works through the server, which then sends edits).
    pub fn execute_command(
        &self,
        path: &Path,
        command: lsp_types::Command,
    ) -> impl Future<Output = Result<(), String>> + use<> {
        let request = self.server_for(path).map(|server| {
            server.request::<lsp_types::request::ExecuteCommand>(lsp_types::ExecuteCommandParams {
                command: command.command,
                arguments: command.arguments.unwrap_or_default(),
                work_done_progress_params: Default::default(),
            })
        });
        async move {
            match request {
                Some(request) => request.await.map(|_| ()),
                None => Err("No language server for this file.".into()),
            }
        }
    }

    /// Problems for a file, as a server would publish them.
    #[cfg(test)]
    pub fn set_diagnostics(&mut self, path: PathBuf, diagnostics: Vec<Diagnostic>) {
        self.diagnostics.insert(path, diagnostics);
        self.diagnostics_version += 1;
    }

    /// The files the servers report an error in (each looked at once, however many).
    pub fn files_with_errors(&self) -> std::collections::HashSet<PathBuf> {
        self.diagnostics
            .iter()
            .filter(|(_, list)| list.iter().any(|d| d.severity == Some(lsp_types::DiagnosticSeverity::ERROR)))
            .map(|(path, _)| path.clone())
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// rust-analyzer's answer for a macro, as it sends it, and no macro there.
    #[test]
    fn a_macro_expansion_is_read() {
        let answer = serde_json::json!({ "name": "println", "expansion": "{ $crate::io::_print(...); }" });
        let expanded: Option<ExpandedMacro> = serde_json::from_value(answer).unwrap();
        assert_eq!(expanded.map(|e| e.name), Some("println".to_string()));
        let none: Option<ExpandedMacro> = serde_json::from_value(serde_json::Value::Null).unwrap();
        assert_eq!(none, None);
        assert_eq!(<ExpandMacro as lsp_types::request::Request>::METHOD, "rust-analyzer/expandMacro");
        // Cargo.toml and the parent module, asked as rust-analyzer reads them.
        let params = OpenCargoTomlParams {
            text_document: TextDocumentIdentifier { uri: "file:///a/src/main.rs".parse().unwrap() },
        };
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            serde_json::json!({ "textDocument": { "uri": "file:///a/src/main.rs" } })
        );
        assert_eq!(<ParentModule as lsp_types::request::Request>::METHOD, "experimental/parentModule");
        // A docs link, as a plain address or with a local one.
        let plain: Option<DocsLink> = serde_json::from_value(serde_json::json!("https://docs.rs/x")).unwrap();
        assert_eq!(plain.and_then(DocsLink::web).as_deref(), Some("https://docs.rs/x"));
        let both: Option<DocsLink> =
            serde_json::from_value(serde_json::json!({ "web": "https://doc.rust-lang.org/std", "local": null }))
                .unwrap();
        assert_eq!(both.and_then(DocsLink::web).as_deref(), Some("https://doc.rust-lang.org/std"));
    }
}
