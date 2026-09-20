use serde_json::json;

use crate::path_guard::resolve_existing_path;
use crate::tool::{
    PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult, WorkingTreeEffect,
};

pub struct GlobTool;

const MAX_MATCHES: usize = 200;
const MAX_OUTPUT_BYTES: usize = 32 * 1024;

#[async_trait::async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::FILE_READ
    }

    fn description(&self) -> &str {
        "Fast file pattern matching. Returns up to 200 matching file paths sorted by modification time, with an omitted count; narrow the pattern when truncated."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files (e.g., '**/*.rs')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to working directory)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("pattern is required and must be a string"),
        };

        let base_dir = match input.get("path").and_then(|v| v.as_str()) {
            Some(path) => match resolve_existing_path(path, ctx) {
                Ok(path) => path,
                Err(err) => return ToolResult::error(err),
            },
            None => match resolve_existing_path(".", ctx) {
                Ok(path) => path,
                Err(err) => return ToolResult::error(err),
            },
        };

        let fs = ctx.fs();
        let matches = if ctx.denied_directory_names.is_empty() {
            fs.glob(&base_dir, pattern).await
        } else {
            bounded_glob(&base_dir, pattern, ctx).await
        };
        let matched = match matches {
            Ok(paths) => paths,
            Err(e) => {
                return ToolResult::error(format!("Invalid glob pattern: {e}"));
            }
        };

        let mut files: Vec<(std::path::PathBuf, Option<u128>)> = Vec::new();
        for path in matched {
            let mtime = fs.metadata(&path).await.ok().and_then(|m| m.modified_nanos);
            files.push((path, mtime));
        }

        // Sort by mtime, newest first. A path whose world reports no time
        // sorts last rather than as the epoch, so "unknown" cannot masquerade
        // as "oldest".
        files.sort_by_key(|(path, mtime)| (std::cmp::Reverse(*mtime), path.clone()));

        if files.is_empty() {
            return ToolResult::success("No files matched the pattern.");
        }

        let mut result = String::new();
        let mut shown = 0;
        for (path, _) in files.iter().take(MAX_MATCHES) {
            let path = path.to_string_lossy();
            let separator = usize::from(shown > 0);
            if result.len() + separator + path.len() > MAX_OUTPUT_BYTES {
                break;
            }
            if shown > 0 {
                result.push('\n');
            }
            result.push_str(&path);
            shown += 1;
        }
        let omitted = files.len() - shown;
        if omitted > 0 {
            result.push_str(&format!(
                "\n\n[glob truncated: {shown} matches shown; {omitted} matches omitted; narrow the path or pattern]"
            ));
        }

        ToolResult::success(result)
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::None
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// Prune before entering directories, rather than filtering stale paths after
// the backend has already traversed every old checkout.
async fn bounded_glob(
    base: &std::path::Path,
    pattern: &str,
    ctx: &ToolContext,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    let full = base.join(pattern);
    let matcher = glob::Pattern::new(&full.to_string_lossy())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let fs = ctx.fs();
    let mut pending = vec![base.to_path_buf()];
    let mut seen = std::collections::BTreeSet::new();
    let mut matches = Vec::new();
    while let Some(dir) = pending.pop() {
        let Ok(canonical) = resolve_existing_path(&dir.to_string_lossy(), ctx) else {
            continue;
        };
        if !seen.insert(canonical) {
            continue;
        }
        if seen.len() > 20_000 {
            return Err(std::io::Error::other(
                "Glob directory limit exceeded; narrow the path",
            ));
        }
        for path in fs.read_dir(&dir).await? {
            if resolve_existing_path(&path.to_string_lossy(), ctx).is_err() {
                continue;
            }
            let Ok(meta) = fs.metadata(&path).await else {
                continue;
            };
            if matcher.matches_path(&path) {
                matches.push(path.clone());
            }
            if meta.is_dir {
                pending.push(path);
            }
        }
    }
    Ok(matches)
}
