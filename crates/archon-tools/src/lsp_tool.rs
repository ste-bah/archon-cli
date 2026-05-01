//! LspTool: implements the Tool trait with 9 LSP operations (TASK-CLI-313).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::Mutex;

use crate::lsp_formatters;
use crate::lsp_manager::LspServerManager;
use crate::lsp_types::{LspInput, LspOperation, LspOutput};
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// LspTool
// ---------------------------------------------------------------------------

/// Provides code intelligence via LSP: 9 operations, read-only, concurrency-safe.
///
/// The tool is disabled when no LSP server is connected (`is_enabled() = false`).
pub struct LspTool {
    manager: Arc<Mutex<LspServerManager>>,
}

impl LspTool {
    pub fn new(manager: Arc<Mutex<LspServerManager>>) -> Self {
        Self { manager }
    }

    /// Returns true if the LSP server is currently connected.
    pub fn is_enabled(&self) -> bool {
        // Try a non-blocking check — if lock is contended, return false (safe fallback)
        if let Ok(guard) = self.manager.try_lock() {
            guard.is_connected()
        } else {
            false
        }
    }

    /// The tool is read-only — it never modifies files.
    pub fn is_read_only(&self) -> bool {
        true
    }

    /// Multiple LSP requests can be in-flight simultaneously.
    pub fn is_concurrency_safe(&self) -> bool {
        true
    }
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        "Code intelligence via LSP: go-to-definition, find-references, hover, \
         document symbols, workspace symbols, go-to-implementation, call hierarchy. \
         Returns empty result when no language server is connected."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": LspOperation::ALL.iter().map(|op| op.as_str()).collect::<Vec<_>>(),
                    "description": "LSP operation to perform"
                },
                "file_path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file"
                },
                "line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-based line number"
                },
                "character": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-based character offset"
                }
            },
            "required": ["operation", "file_path", "line", "character"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let lsp_input: LspInput = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(e) => return ToolResult::error(format!("invalid input: {e}")),
        };

        let file_path = if std::path::Path::new(&lsp_input.file_path).is_absolute() {
            lsp_input.file_path.clone()
        } else {
            ctx.working_dir
                .join(&lsp_input.file_path)
                .to_string_lossy()
                .into_owned()
        };

        let mut guard = self.manager.lock().await;

        // Lazy init on first use
        if !guard.is_connected()
            && let Err(e) = guard.ensure_connected().await {
                return ToolResult::error(format!("LSP not available: {e}"));
            }

        let client = match guard.client_mut() {
            Some(c) => c,
            None => return ToolResult::error("LSP client unexpectedly disconnected"),
        };

        let output = match lsp_input.operation {
            LspOperation::GoToDefinition => {
                match client
                    .go_to_definition(&file_path, lsp_input.line, lsp_input.character)
                    .await
                {
                    Ok(Some(resp)) => {
                        let (result, result_count, file_count) =
                            lsp_formatters::format_go_to_definition(&resp);
                        LspOutput {
                            operation: "goToDefinition".into(),
                            result,
                            file_path: lsp_input.file_path,
                            result_count,
                            file_count,
                        }
                    }
                    Ok(None) => no_result("goToDefinition", &lsp_input.file_path),
                    Err(e) => return ToolResult::error(e.to_string()),
                }
            }
            LspOperation::FindReferences => {
                match client
                    .find_references(&file_path, lsp_input.line, lsp_input.character)
                    .await
                {
                    Ok(Some(locs)) => {
                        let (result, result_count, file_count) =
                            lsp_formatters::format_find_references(&locs);
                        LspOutput {
                            operation: "findReferences".into(),
                            result,
                            file_path: lsp_input.file_path,
                            result_count,
                            file_count,
                        }
                    }
                    Ok(None) => no_result("findReferences", &lsp_input.file_path),
                    Err(e) => return ToolResult::error(e.to_string()),
                }
            }
            LspOperation::Hover => {
                match client
                    .hover(&file_path, lsp_input.line, lsp_input.character)
                    .await
                {
                    Ok(Some(hover)) => {
                        let result = lsp_formatters::format_hover(&hover);
                        LspOutput {
                            operation: "hover".into(),
                            result,
                            file_path: lsp_input.file_path,
                            result_count: 1,
                            file_count: 1,
                        }
                    }
                    Ok(None) => no_result("hover", &lsp_input.file_path),
                    Err(e) => return ToolResult::error(e.to_string()),
                }
            }
            LspOperation::DocumentSymbol => match client.document_symbol(&file_path).await {
                Ok(Some(resp)) => {
                    let (result, result_count) = match resp {
                        lsp_types::DocumentSymbolResponse::Flat(syms) => {
                            let count = syms.len();
                            (lsp_formatters::format_document_symbols_flat(&syms), count)
                        }
                        lsp_types::DocumentSymbolResponse::Nested(syms) => {
                            let count = syms.len();
                            (
                                lsp_formatters::format_document_symbols_nested(&syms, 0),
                                count,
                            )
                        }
                    };
                    LspOutput {
                        operation: "documentSymbol".into(),
                        result,
                        file_path: lsp_input.file_path,
                        result_count,
                        file_count: 1,
                    }
                }
                Ok(None) => no_result("documentSymbol", &lsp_input.file_path),
                Err(e) => return ToolResult::error(e.to_string()),
            },
            LspOperation::WorkspaceSymbol => {
                // Use file_path as the query string for workspace symbol search
                let query = &lsp_input.file_path;
                match client.workspace_symbol(query).await {
                    Ok(Some(resp)) => {
                        let syms: Vec<lsp_types::SymbolInformation> = match resp {
                            lsp_types::WorkspaceSymbolResponse::Flat(s) => s,
                            lsp_types::WorkspaceSymbolResponse::Nested(_) => vec![],
                        };
                        let (result, result_count, file_count) =
                            lsp_formatters::format_workspace_symbols(&syms);
                        LspOutput {
                            operation: "workspaceSymbol".into(),
                            result,
                            file_path: lsp_input.file_path,
                            result_count,
                            file_count,
                        }
                    }
                    Ok(None) => no_result("workspaceSymbol", &lsp_input.file_path),
                    Err(e) => return ToolResult::error(e.to_string()),
                }
            }
            LspOperation::GoToImplementation => {
                match client
                    .go_to_implementation(&file_path, lsp_input.line, lsp_input.character)
                    .await
                {
                    Ok(Some(resp)) => {
                        let locations: Vec<lsp_types::Location> = match resp {
                            lsp_types::GotoDefinitionResponse::Scalar(loc) => vec![loc],
                            lsp_types::GotoDefinitionResponse::Array(locs) => locs,
                            lsp_types::GotoDefinitionResponse::Link(links) => links
                                .into_iter()
                                .map(|l| lsp_types::Location {
                                    uri: l.target_uri,
                                    range: l.target_range,
                                })
                                .collect(),
                        };
                        let (result, result_count, file_count) =
                            lsp_formatters::format_go_to_implementation(&locations);
                        LspOutput {
                            operation: "goToImplementation".into(),
                            result,
                            file_path: lsp_input.file_path,
                            result_count,
                            file_count,
                        }
                    }
                    Ok(None) => no_result("goToImplementation", &lsp_input.file_path),
                    Err(e) => return ToolResult::error(e.to_string()),
                }
            }
            LspOperation::PrepareCallHierarchy => {
                match client
                    .prepare_call_hierarchy(&file_path, lsp_input.line, lsp_input.character)
                    .await
                {
                    Ok(Some(items)) => {
                        let result = lsp_formatters::format_prepare_call_hierarchy(&items);
                        let count = items.len();
                        LspOutput {
                            operation: "prepareCallHierarchy".into(),
                            result,
                            file_path: lsp_input.file_path,
                            result_count: count,
                            file_count: count,
                        }
                    }
                    Ok(None) => no_result("prepareCallHierarchy", &lsp_input.file_path),
                    Err(e) => return ToolResult::error(e.to_string()),
                }
            }
            LspOperation::IncomingCalls => {
                // For incomingCalls/outgoingCalls, we first need to prepareCallHierarchy
                match client
                    .prepare_call_hierarchy(&file_path, lsp_input.line, lsp_input.character)
                    .await
                {
                    Ok(Some(items)) if !items.is_empty() => {
                        match client
                            .incoming_calls(items.into_iter().next().unwrap())
                            .await
                        {
                            Ok(Some(calls)) => {
                                let (result, result_count, file_count) =
                                    lsp_formatters::format_incoming_calls(&calls);
                                LspOutput {
                                    operation: "incomingCalls".into(),
                                    result,
                                    file_path: lsp_input.file_path,
                                    result_count,
                                    file_count,
                                }
                            }
                            Ok(None) => no_result("incomingCalls", &lsp_input.file_path),
                            Err(e) => return ToolResult::error(e.to_string()),
                        }
                    }
                    _ => no_result("incomingCalls", &lsp_input.file_path),
                }
            }
            LspOperation::OutgoingCalls => {
                match client
                    .prepare_call_hierarchy(&file_path, lsp_input.line, lsp_input.character)
                    .await
                {
                    Ok(Some(items)) if !items.is_empty() => {
                        match client
                            .outgoing_calls(items.into_iter().next().unwrap())
                            .await
                        {
                            Ok(Some(calls)) => {
                                let (result, result_count, file_count) =
                                    lsp_formatters::format_outgoing_calls(&calls);
                                LspOutput {
                                    operation: "outgoingCalls".into(),
                                    result,
                                    file_path: lsp_input.file_path,
                                    result_count,
                                    file_count,
                                }
                            }
                            Ok(None) => no_result("outgoingCalls", &lsp_input.file_path),
                            Err(e) => return ToolResult::error(e.to_string()),
                        }
                    }
                    _ => no_result("outgoingCalls", &lsp_input.file_path),
                }
            }
        };

        ToolResult::success(serde_json::to_string_pretty(&output).unwrap_or_default())
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

fn no_result(operation: &str, file_path: &str) -> LspOutput {
    LspOutput {
        operation: operation.to_string(),
        result: format!("No {} result.", operation),
        file_path: file_path.to_string(),
        result_count: 0,
        file_count: 0,
    }
}
