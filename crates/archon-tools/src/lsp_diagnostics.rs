//! LSP diagnostic registry for TASK-CLI-313.
//!
//! `publishDiagnostics` notifications from the LSP server are cached here.
//! Diagnostics are NOT a tool operation — they are stored and can be queried
//! by other Archon components.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// DiagnosticSeverity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

// ---------------------------------------------------------------------------
// LspDiagnostic
// ---------------------------------------------------------------------------

/// A single LSP diagnostic (flattened from lsp_types::Diagnostic).
#[derive(Debug, Clone)]
pub struct LspDiagnostic {
    pub message: String,
    pub severity: DiagnosticSeverity,
    pub range_start_line: u32,
    pub range_start_char: u32,
    pub range_end_line: u32,
    pub range_end_char: u32,
    pub code: Option<String>,
    pub source: Option<String>,
}

// ---------------------------------------------------------------------------
// LspDiagnosticRegistry
// ---------------------------------------------------------------------------

/// Thread-safe cache of diagnostics per file URI.
///
/// Updated by the mainloop when the LSP server publishes diagnostics.
/// External code queries via `get_diagnostics(file_path)`.
#[derive(Debug, Default)]
pub struct LspDiagnosticRegistry {
    /// Keyed by normalized file path string.
    diagnostics: HashMap<String, Vec<LspDiagnostic>>,
}

impl LspDiagnosticRegistry {
    pub fn new() -> Self {
        Self {
            diagnostics: HashMap::new(),
        }
    }

    /// Replace diagnostics for a file (mirrors LSP `publishDiagnostics` semantics).
    ///
    /// An empty `Vec` clears the diagnostics for that file.
    pub fn publish(&mut self, file_path: &str, diagnostics: Vec<LspDiagnostic>) {
        if diagnostics.is_empty() {
            self.diagnostics.remove(file_path);
        } else {
            self.diagnostics.insert(file_path.to_string(), diagnostics);
        }
    }

    /// Get all diagnostics for a file. Returns empty slice if no diagnostics recorded.
    pub fn get_diagnostics(&self, file_path: &str) -> &[LspDiagnostic] {
        self.diagnostics
            .get(file_path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Clear all diagnostics for a file.
    pub fn clear(&mut self, file_path: &str) {
        self.diagnostics.remove(file_path);
    }

    /// Number of files with active diagnostics.
    pub fn file_count(&self) -> usize {
        self.diagnostics.len()
    }
}

// ---------------------------------------------------------------------------
// Conversion from lsp_types::Diagnostic
// ---------------------------------------------------------------------------

impl LspDiagnostic {
    pub fn from_lsp(d: &lsp_types::Diagnostic) -> Self {
        Self {
            message: d.message.clone(),
            severity: d
                .severity
                .map(|s| match s {
                    lsp_types::DiagnosticSeverity::ERROR => DiagnosticSeverity::Error,
                    lsp_types::DiagnosticSeverity::WARNING => DiagnosticSeverity::Warning,
                    lsp_types::DiagnosticSeverity::INFORMATION => DiagnosticSeverity::Information,
                    lsp_types::DiagnosticSeverity::HINT => DiagnosticSeverity::Hint,
                    _ => DiagnosticSeverity::Information,
                })
                .unwrap_or(DiagnosticSeverity::Error),
            range_start_line: d.range.start.line,
            range_start_char: d.range.start.character,
            range_end_line: d.range.end.line,
            range_end_char: d.range.end.character,
            code: d.code.as_ref().map(|c| match c {
                lsp_types::NumberOrString::Number(n) => n.to_string(),
                lsp_types::NumberOrString::String(s) => s.clone(),
            }),
            source: d.source.clone(),
        }
    }
}
