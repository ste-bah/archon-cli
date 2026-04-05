//! LSP protocol client for TASK-CLI-313.
//!
//! Uses `async-lsp` (JSON-RPC 2.0 over stdio) to communicate with language servers.
//! The client is async and manages file synchronization, request dispatch, and
//! push-based diagnostics via `LspDiagnosticRegistry`.

use std::collections::HashSet;
use std::ops::ControlFlow;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_lsp::concurrency::ConcurrencyLayer;
use async_lsp::router::Router;
use async_lsp::{LanguageServer, MainLoop};
use lsp_types::notification::{PublishDiagnostics, ShowMessage};
use lsp_types::request::GotoImplementationParams;
use lsp_types::{
    ClientCapabilities, DidCloseTextDocumentParams, DidOpenTextDocumentParams, InitializeParams,
    InitializedParams, TextDocumentIdentifier, TextDocumentItem, Url, WorkspaceFolder,
};
use tokio::process::{Child, Command};
use tokio::time::timeout;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tower::ServiceBuilder;

use crate::lsp_diagnostics::{LspDiagnostic, LspDiagnosticRegistry};

// ---------------------------------------------------------------------------
// LspError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum LspError {
    #[error("no language server detected for this project")]
    NoServerDetected,
    #[error("invalid project root path")]
    InvalidProjectRoot,
    #[error("server binary not found: {0}")]
    BinaryNotFound(String),
    #[error("initialization timed out after {0:?}")]
    InitTimeout(Duration),
    #[error("request timed out after {0:?}")]
    RequestTimeout(Duration),
    #[error("LSP error: {0}")]
    Protocol(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Internal event type for stopping the mainloop
// ---------------------------------------------------------------------------

struct Stop;

// ---------------------------------------------------------------------------
// Shared state between mainloop and LspClient
// ---------------------------------------------------------------------------

struct ClientSharedState {
    diagnostics: Arc<RwLock<LspDiagnosticRegistry>>,
}

// ---------------------------------------------------------------------------
// LspClient
// ---------------------------------------------------------------------------

/// Active connection to a language server.
pub struct LspClient {
    /// Handle to send requests to the server.
    server: async_lsp::ServerSocket,
    /// Background task running the LSP mainloop.
    _mainloop_handle: tokio::task::JoinHandle<()>,
    /// The spawned language server process.
    _child: Child,
    /// Per-request timeout.
    request_timeout: Duration,
    /// Files currently open for synchronization.
    open_files: HashSet<Url>,
    /// Shared diagnostic registry (also held by the mainloop).
    pub diagnostics: Arc<RwLock<LspDiagnosticRegistry>>,
}

impl LspClient {
    /// Connect to a language server and initialize the LSP session.
    pub async fn connect(
        binary: &str,
        args: &[&str],
        root_uri: Url,
        init_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, LspError> {
        // Verify binary is on PATH before trying to spawn
        if which::which(binary).is_err() {
            return Err(LspError::BinaryNotFound(binary.to_string()));
        }

        let diagnostics = Arc::new(RwLock::new(LspDiagnosticRegistry::new()));
        let diagnostics_clone = Arc::clone(&diagnostics);

        let (mainloop, mut server) = MainLoop::new_client(|_server_socket| {
            let diag_registry = Arc::clone(&diagnostics_clone);
            let mut router = Router::new(ClientSharedState {
                diagnostics: diag_registry,
            });

            router
                .notification::<PublishDiagnostics>(|state, params| {
                    let file_path = params.uri.path().to_string();
                    let lsp_diags: Vec<LspDiagnostic> = params
                        .diagnostics
                        .iter()
                        .map(LspDiagnostic::from_lsp)
                        .collect();
                    if let Ok(mut registry) = state.diagnostics.write() {
                        registry.publish(&file_path, lsp_diags);
                    }
                    ControlFlow::Continue(())
                })
                .notification::<ShowMessage>(|_, params| {
                    tracing::info!("[lsp] {:?}: {}", params.typ, params.message);
                    ControlFlow::Continue(())
                })
                .event(|_, _: Stop| ControlFlow::Break(Ok(())));

            ServiceBuilder::new()
                .layer(ConcurrencyLayer::default())
                .service(router)
        });

        // Spawn the server process
        let mut child = Command::new(binary)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LspError::Protocol("process stdout not piped".into()))?
            .compat();
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LspError::Protocol("process stdin not piped".into()))?
            .compat_write();

        let mainloop_handle = tokio::spawn(async move {
            let _ = mainloop.run_buffered(stdout, stdin).await;
        });

        // Initialize
        let init_fut = server.initialize(InitializeParams {
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri.clone(),
                name: "root".into(),
            }]),
            capabilities: ClientCapabilities::default(),
            ..InitializeParams::default()
        });

        timeout(init_timeout, init_fut)
            .await
            .map_err(|_| LspError::InitTimeout(init_timeout))?
            .map_err(|e| LspError::Protocol(e.to_string()))?;

        server
            .initialized(InitializedParams {})
            .map_err(|e| LspError::Protocol(e.to_string()))?;

        Ok(Self {
            server,
            _mainloop_handle: mainloop_handle,
            _child: child,
            request_timeout,
            open_files: HashSet::new(),
            diagnostics,
        })
    }

    /// Ensure a file is open for synchronization.
    pub async fn ensure_file_open(&mut self, file_path: &str) -> Result<Url, LspError> {
        let uri = Url::from_file_path(file_path)
            .map_err(|_| LspError::Protocol(format!("invalid file path: {}", file_path)))?;

        if !self.open_files.contains(&uri) {
            let text = std::fs::read_to_string(file_path)
                .map_err(|e| LspError::Io(e))?;

            let language_id = detect_language_id(file_path);

            self.server
                .did_open(DidOpenTextDocumentParams {
                    text_document: TextDocumentItem {
                        uri: uri.clone(),
                        language_id: language_id.to_string(),
                        version: 0,
                        text,
                    },
                })
                .map_err(|e| LspError::Protocol(e.to_string()))?;

            self.open_files.insert(uri.clone());
        }
        Ok(uri)
    }

    /// Close an open file.
    pub fn close_file(&mut self, uri: &Url) {
        if self.open_files.remove(uri) {
            let _ = self.server.did_close(DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
            });
        }
    }

    // ── LSP request helpers ─────────────────────────────────────────────────

    /// Run an LSP request with the per-client timeout.
    async fn request_with_timeout<F, T>(&self, fut: F) -> Result<T, LspError>
    where
        F: std::future::Future<Output = Result<T, async_lsp::Error>>,
    {
        timeout(self.request_timeout, fut)
            .await
            .map_err(|_| LspError::RequestTimeout(self.request_timeout))?
            .map_err(|e| LspError::Protocol(e.to_string()))
    }

    // ── goToDefinition ──────────────────────────────────────────────────────

    pub async fn go_to_definition(
        &mut self,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::GotoDefinitionResponse>, LspError> {
        let uri = self.ensure_file_open(file_path).await?;
        let params = lsp_types::GotoDefinitionParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position::new(line.saturating_sub(1), character.saturating_sub(1)),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.request_with_timeout(self.server.clone().definition(params)).await
    }

    // ── findReferences ──────────────────────────────────────────────────────

    pub async fn find_references(
        &mut self,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<Vec<lsp_types::Location>>, LspError> {
        let uri = self.ensure_file_open(file_path).await?;
        let params = lsp_types::ReferenceParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position::new(line.saturating_sub(1), character.saturating_sub(1)),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp_types::ReferenceContext {
                include_declaration: true,
            },
        };
        self.request_with_timeout(self.server.clone().references(params)).await
    }

    // ── hover ───────────────────────────────────────────────────────────────

    pub async fn hover(
        &mut self,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::Hover>, LspError> {
        let uri = self.ensure_file_open(file_path).await?;
        let params = lsp_types::HoverParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position::new(line.saturating_sub(1), character.saturating_sub(1)),
            },
            work_done_progress_params: Default::default(),
        };
        self.request_with_timeout(self.server.clone().hover(params)).await
    }

    // ── documentSymbol ──────────────────────────────────────────────────────

    pub async fn document_symbol(
        &mut self,
        file_path: &str,
    ) -> Result<Option<lsp_types::DocumentSymbolResponse>, LspError> {
        let uri = self.ensure_file_open(file_path).await?;
        let params = lsp_types::DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.request_with_timeout(self.server.clone().document_symbol(params)).await
    }

    // ── workspaceSymbol ─────────────────────────────────────────────────────

    pub async fn workspace_symbol(
        &mut self,
        query: &str,
    ) -> Result<Option<lsp_types::WorkspaceSymbolResponse>, LspError> {
        let params = lsp_types::WorkspaceSymbolParams {
            query: query.to_string(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.request_with_timeout(self.server.clone().symbol(params)).await
    }

    // ── goToImplementation ──────────────────────────────────────────────────

    pub async fn go_to_implementation(
        &mut self,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::GotoDefinitionResponse>, LspError> {
        let uri = self.ensure_file_open(file_path).await?;
        let params = GotoImplementationParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position::new(line.saturating_sub(1), character.saturating_sub(1)),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.request_with_timeout(self.server.clone().implementation(params)).await
    }

    // ── prepareCallHierarchy ────────────────────────────────────────────────

    pub async fn prepare_call_hierarchy(
        &mut self,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyItem>>, LspError> {
        let uri = self.ensure_file_open(file_path).await?;
        let params = lsp_types::CallHierarchyPrepareParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position::new(line.saturating_sub(1), character.saturating_sub(1)),
            },
            work_done_progress_params: Default::default(),
        };
        self.request_with_timeout(self.server.clone().prepare_call_hierarchy(params)).await
    }

    // ── incomingCalls ───────────────────────────────────────────────────────

    pub async fn incoming_calls(
        &mut self,
        item: lsp_types::CallHierarchyItem,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyIncomingCall>>, LspError> {
        let params = lsp_types::CallHierarchyIncomingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.request_with_timeout(self.server.clone().incoming_calls(params)).await
    }

    // ── outgoingCalls ───────────────────────────────────────────────────────

    pub async fn outgoing_calls(
        &mut self,
        item: lsp_types::CallHierarchyItem,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyOutgoingCall>>, LspError> {
        let params = lsp_types::CallHierarchyOutgoingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.request_with_timeout(self.server.clone().outgoing_calls(params)).await
    }

    // ── Lifecycle ───────────────────────────────────────────────────────────

    /// Shut down the LSP server gracefully.
    pub async fn shutdown(mut self) -> Result<(), LspError> {
        let _ = timeout(
            Duration::from_secs(5),
            self.server.shutdown(()),
        )
        .await;
        let _ = self.server.exit(());
        let _ = self.server.emit(Stop);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Language ID detection
// ---------------------------------------------------------------------------

fn detect_language_id(file_path: &str) -> &'static str {
    let path = std::path::Path::new(file_path);
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") => "javascript",
        Some("py") => "python",
        Some("go") => "go",
        Some("java") => "java",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") => "cpp",
        _ => "plaintext",
    }
}
