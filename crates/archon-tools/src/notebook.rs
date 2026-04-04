use std::fs;
use std::path::Path;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool for editing Jupyter notebook (.ipynb) cells.
///
/// Supports insert, replace, delete, and move operations on notebook cells.
/// Writes are atomic (tmp file + rename).
pub struct NotebookEditTool;

#[async_trait::async_trait]
impl Tool for NotebookEditTool {
    fn name(&self) -> &str {
        "NotebookEdit"
    }

    fn description(&self) -> &str {
        "Edit Jupyter notebook (.ipynb) cells. Supports insert, replace, delete, and move operations."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the .ipynb notebook file"
                },
                "command": {
                    "type": "string",
                    "enum": ["insert", "replace", "delete", "move"],
                    "description": "The operation to perform on a cell"
                },
                "cell_index": {
                    "type": "integer",
                    "description": "Index of the cell to operate on (0-based)"
                },
                "content": {
                    "type": "string",
                    "description": "Cell content (required for insert and replace)"
                },
                "cell_type": {
                    "type": "string",
                    "enum": ["code", "markdown", "raw"],
                    "description": "Cell type (required for insert, default: code)"
                },
                "target_index": {
                    "type": "integer",
                    "description": "Target index for move operation"
                }
            },
            "required": ["path", "command", "cell_index"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let path_str = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("path is required and must be a string"),
        };

        let path = Path::new(path_str);

        // Validate .ipynb extension
        match path.extension().and_then(|e| e.to_str()) {
            Some("ipynb") => {}
            _ => return ToolResult::error("path must have .ipynb extension"),
        }

        let command = match input.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::error("command is required and must be a string"),
        };

        let cell_index = match input.get("cell_index").and_then(|v| v.as_u64()) {
            Some(i) => i as usize,
            None => return ToolResult::error("cell_index is required and must be a non-negative integer"),
        };

        // Read and parse notebook
        let raw = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Failed to read notebook: {e}")),
        };

        let mut notebook: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return ToolResult::error(format!("Failed to parse notebook JSON: {e}")),
        };

        let cells = match notebook.get_mut("cells").and_then(|v| v.as_array_mut()) {
            Some(c) => c,
            None => return ToolResult::error("Notebook has no 'cells' array"),
        };

        let result_msg = match command {
            "insert" => execute_insert(cells, cell_index, &input),
            "replace" => execute_replace(cells, cell_index, &input),
            "delete" => execute_delete(cells, cell_index),
            "move" => execute_move(cells, cell_index, &input),
            other => Err(format!("Unknown command: '{other}'. Use insert, replace, delete, or move.")),
        };

        let msg = match result_msg {
            Ok(m) => m,
            Err(e) => return ToolResult::error(e),
        };

        // Atomic write: write to tmp, then rename
        let tmp_path = path.with_extension("notebook.tmp");
        let serialized = match serde_json::to_string_pretty(&notebook) {
            Ok(s) => s,
            Err(e) => return ToolResult::error(format!("Failed to serialize notebook: {e}")),
        };

        if let Err(e) = fs::write(&tmp_path, &serialized) {
            return ToolResult::error(format!("Failed to write temp file: {e}"));
        }

        if let Err(e) = fs::rename(&tmp_path, path) {
            // Clean up tmp file on rename failure
            let _ = fs::remove_file(&tmp_path);
            return ToolResult::error(format!("Failed to rename temp file: {e}"));
        }

        ToolResult::success(msg)
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

fn execute_insert(
    cells: &mut Vec<serde_json::Value>,
    index: usize,
    input: &serde_json::Value,
) -> Result<String, String> {
    if index > cells.len() {
        return Err(format!(
            "cell_index {index} is out of range (notebook has {} cells, valid insert range: 0..={})",
            cells.len(),
            cells.len()
        ));
    }

    let content = input
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "content is required for insert".to_string())?;

    let cell_type = input
        .get("cell_type")
        .and_then(|v| v.as_str())
        .unwrap_or("code");

    let mut cell = json!({
        "cell_type": cell_type,
        "source": [content],
        "metadata": {}
    });

    // Add code-specific fields
    if cell_type == "code" {
        cell["execution_count"] = serde_json::Value::Null;
        cell["outputs"] = json!([]);
    }

    cells.insert(index, cell);
    Ok(format!("Inserted {cell_type} cell at index {index}"))
}

fn execute_replace(
    cells: &mut Vec<serde_json::Value>,
    index: usize,
    input: &serde_json::Value,
) -> Result<String, String> {
    if index >= cells.len() {
        return Err(format!(
            "cell_index {index} is out of range (notebook has {} cells)",
            cells.len()
        ));
    }

    let content = input
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "content is required for replace".to_string())?;

    // Only replace the source; preserve cell_type, metadata, outputs, etc.
    cells[index]["source"] = json!([content]);

    Ok(format!("Replaced content of cell at index {index}"))
}

fn execute_delete(
    cells: &mut Vec<serde_json::Value>,
    index: usize,
) -> Result<String, String> {
    if index >= cells.len() {
        return Err(format!(
            "cell_index {index} is out of range (notebook has {} cells)",
            cells.len()
        ));
    }

    cells.remove(index);
    Ok(format!("Deleted cell at index {index}"))
}

fn execute_move(
    cells: &mut Vec<serde_json::Value>,
    from: usize,
    input: &serde_json::Value,
) -> Result<String, String> {
    let to = input
        .get("target_index")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "target_index is required for move".to_string())? as usize;

    if from >= cells.len() {
        return Err(format!(
            "cell_index {from} is out of range (notebook has {} cells)",
            cells.len()
        ));
    }
    if to >= cells.len() {
        return Err(format!(
            "target_index {to} is out of range (notebook has {} cells)",
            cells.len()
        ));
    }

    let cell = cells.remove(from);
    cells.insert(to, cell);
    Ok(format!("Moved cell from index {from} to {to}"))
}
