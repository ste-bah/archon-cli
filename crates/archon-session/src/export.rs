//! Session export: convert conversation messages to Markdown, JSON, or plain text.

use serde_json::json;

// ---------------------------------------------------------------------------
// ExportFormat
// ---------------------------------------------------------------------------

/// Supported export formats for session conversations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Json,
    Text,
}

impl ExportFormat {
    /// Parse a format string (case-insensitive).
    ///
    /// Accepts `"markdown"`, `"json"`, or `"text"`.
    /// Returns an error message for anything else.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "markdown" | "md" => Ok(Self::Markdown),
            "json" => Ok(Self::Json),
            "text" | "txt" => Ok(Self::Text),
            other => Err(format!(
                "unknown export format '{other}': expected 'markdown', 'json', or 'text'"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Options for session export.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Include thinking blocks in the export.
    pub include_thinking: bool,
    /// Maximum characters of tool output to include per tool call.
    pub tool_output_max: usize,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_thinking: false,
            tool_output_max: 500,
        }
    }
}

/// Session metadata for export headers.
#[derive(Debug, Clone, Default)]
pub struct ExportMetadata {
    pub git_branch: Option<String>,
    pub working_directory: Option<String>,
    pub start_time: Option<String>,
    pub model: Option<String>,
}

/// Export session messages in the requested format.
///
/// Each message is expected to be a JSON object with at least `"role"` and
/// `"content"` fields. Unknown roles are rendered literally.
pub fn export_session(
    messages: &[serde_json::Value],
    session_id: &str,
    format: ExportFormat,
) -> Result<String, String> {
    export_session_with_options(
        messages,
        session_id,
        format,
        &ExportOptions::default(),
        &ExportMetadata::default(),
    )
}

/// Export with full options and metadata.
pub fn export_session_with_options(
    messages: &[serde_json::Value],
    session_id: &str,
    format: ExportFormat,
    _opts: &ExportOptions,
    meta: &ExportMetadata,
) -> Result<String, String> {
    match format {
        ExportFormat::Markdown => export_markdown(messages, session_id, meta),
        ExportFormat::Json => export_json(messages, session_id, meta),
        ExportFormat::Text => export_text(messages, session_id, meta),
    }
}

/// Generate a default export filename.
pub fn default_export_filename(session_id: &str, format: ExportFormat) -> String {
    let ext = match format {
        ExportFormat::Markdown => "md",
        ExportFormat::Json => "json",
        ExportFormat::Text => "txt",
    };
    let short_id: String = session_id.chars().take(8).collect();
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    format!("archon-export-{short_id}-{timestamp}.{ext}")
}

/// Write export to a file, returning the path.
pub fn write_export(content: &str, output_path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create directory: {e}"))?;
    }
    std::fs::write(output_path, content).map_err(|e| format!("failed to write export file: {e}"))
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

fn export_markdown(
    messages: &[serde_json::Value],
    session_id: &str,
    meta: &ExportMetadata,
) -> Result<String, String> {
    let mut out = format!("# Session {session_id}\n\n");

    // Metadata header
    if let Some(ref dir) = meta.working_directory {
        out.push_str(&format!("- **Directory**: {dir}\n"));
    }
    if let Some(ref branch) = meta.git_branch {
        out.push_str(&format!("- **Branch**: {branch}\n"));
    }
    if let Some(ref model) = meta.model {
        out.push_str(&format!("- **Model**: {model}\n"));
    }
    if let Some(ref start) = meta.start_time {
        out.push_str(&format!("- **Started**: {start}\n"));
    }
    if meta.working_directory.is_some() || meta.git_branch.is_some() {
        out.push('\n');
    }

    let mut turn = 0usize;
    for msg in messages {
        let role = msg
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let content = extract_content(msg);
        let timestamp = msg.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
        let tool_summary = extract_tool_summary(msg);

        let label = role_label(role);

        turn += 1;
        out.push_str(&format!("## Turn {turn}\n\n"));
        if !timestamp.is_empty() {
            out.push_str(&format!("*{timestamp}*\n\n"));
        }
        out.push_str(&format!("**{label}**: {content}\n\n"));
        if !tool_summary.is_empty() {
            out.push_str(&format!("{tool_summary}\n\n"));
        }
        out.push_str("---\n\n");
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

fn export_json(
    messages: &[serde_json::Value],
    session_id: &str,
    meta: &ExportMetadata,
) -> Result<String, String> {
    let doc = json!({
        "session_id": session_id,
        "message_count": messages.len(),
        "working_directory": meta.working_directory,
        "git_branch": meta.git_branch,
        "model": meta.model,
        "start_time": meta.start_time,
        "messages": messages,
    });

    serde_json::to_string_pretty(&doc).map_err(|e| format!("JSON serialization failed: {e}"))
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

fn export_text(
    messages: &[serde_json::Value],
    session_id: &str,
    meta: &ExportMetadata,
) -> Result<String, String> {
    let mut out = format!("Session: {session_id}\n");

    if let Some(ref dir) = meta.working_directory {
        out.push_str(&format!("Directory: {dir}\n"));
    }
    if let Some(ref branch) = meta.git_branch {
        out.push_str(&format!("Branch: {branch}\n"));
    }
    out.push('\n');

    for msg in messages {
        let role = msg
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let content = extract_content(msg);
        let timestamp = msg.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
        let label = role_label(role);
        if !timestamp.is_empty() {
            out.push_str(&format!("[{timestamp}] "));
        }
        out.push_str(&format!("{label}: {content}\n"));
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a role string into a display label.
fn role_label(role: &str) -> &str {
    match role {
        "user" => "User",
        "assistant" => "Assistant",
        "system" => "System",
        _ => role,
    }
}

/// Extract a one-line summary of tool calls from a message.
///
/// Returns empty string if no tool_use blocks are present.
fn extract_tool_summary(msg: &serde_json::Value) -> String {
    let content = match msg.get("content") {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => return String::new(),
    };

    let mut summaries = Vec::new();
    for item in content {
        if item.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            summaries.push(format!("> Tool: {name}"));
        }
        if item.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
            let is_error = item
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let status = if is_error { "error" } else { "ok" };
            summaries.push(format!("> Result: {status}"));
        }
    }
    summaries.join("\n")
}

/// Extract the text content from a message value.
///
/// Handles both `"content": "string"` and `"content": [{ "text": "..." }]`
/// formats that various providers use.
fn extract_content(msg: &serde_json::Value) -> String {
    match msg.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(arr)) => {
            let mut parts = Vec::new();
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    parts.push(text.to_string());
                }
            }
            parts.join("\n")
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_roundtrip() {
        assert!(matches!(
            ExportFormat::from_str("markdown"),
            Ok(ExportFormat::Markdown)
        ));
        assert!(matches!(
            ExportFormat::from_str("json"),
            Ok(ExportFormat::Json)
        ));
        assert!(matches!(
            ExportFormat::from_str("text"),
            Ok(ExportFormat::Text)
        ));
    }

    #[test]
    fn format_aliases() {
        assert!(matches!(
            ExportFormat::from_str("md"),
            Ok(ExportFormat::Markdown)
        ));
        assert!(matches!(
            ExportFormat::from_str("txt"),
            Ok(ExportFormat::Text)
        ));
    }

    #[test]
    fn role_label_mapping() {
        assert_eq!(role_label("user"), "User");
        assert_eq!(role_label("assistant"), "Assistant");
        assert_eq!(role_label("system"), "System");
        assert_eq!(role_label("custom"), "custom");
    }
}
