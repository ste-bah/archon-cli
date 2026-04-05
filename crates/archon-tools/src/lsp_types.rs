//! LSP input/output types for TASK-CLI-313.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// LspOperation enum — exactly 9 operations (no diagnostics, no completions)
// ---------------------------------------------------------------------------

/// All supported LSP operations.
///
/// `diagnostics` and `completions` are intentionally excluded — they are push-based,
/// not request-based, and are not in the reference implementation's tool operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LspOperation {
    GoToDefinition,
    FindReferences,
    Hover,
    DocumentSymbol,
    WorkspaceSymbol,
    GoToImplementation,
    PrepareCallHierarchy,
    IncomingCalls,
    OutgoingCalls,
}

impl LspOperation {
    /// All operation variants for validation and count assertions.
    pub const ALL: &'static [LspOperation] = &[
        LspOperation::GoToDefinition,
        LspOperation::FindReferences,
        LspOperation::Hover,
        LspOperation::DocumentSymbol,
        LspOperation::WorkspaceSymbol,
        LspOperation::GoToImplementation,
        LspOperation::PrepareCallHierarchy,
        LspOperation::IncomingCalls,
        LspOperation::OutgoingCalls,
    ];

    /// String name as it appears in the JSON schema.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GoToDefinition => "goToDefinition",
            Self::FindReferences => "findReferences",
            Self::Hover => "hover",
            Self::DocumentSymbol => "documentSymbol",
            Self::WorkspaceSymbol => "workspaceSymbol",
            Self::GoToImplementation => "goToImplementation",
            Self::PrepareCallHierarchy => "prepareCallHierarchy",
            Self::IncomingCalls => "incomingCalls",
            Self::OutgoingCalls => "outgoingCalls",
        }
    }
}

// ---------------------------------------------------------------------------
// LspInput — tool input schema
// ---------------------------------------------------------------------------

/// Input to the LSP tool.
///
/// All operations share the same fields; `character` is only relevant for
/// position-based operations (ignored for `workspaceSymbol`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspInput {
    /// Which LSP operation to perform.
    pub operation: LspOperation,
    /// Absolute or relative file path.
    pub file_path: String,
    /// 1-based line number (matching editor conventions).
    pub line: u32,
    /// 1-based character offset.
    pub character: u32,
}

// ---------------------------------------------------------------------------
// LspOutput — tool output schema
// ---------------------------------------------------------------------------

/// Structured result from an LSP operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspOutput {
    /// Operation that was performed.
    pub operation: String,
    /// Human-readable formatted result.
    pub result: String,
    /// Input file path for reference.
    pub file_path: String,
    /// Number of result items returned.
    pub result_count: usize,
    /// Number of distinct files referenced in results.
    pub file_count: usize,
}
