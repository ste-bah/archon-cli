use std::fs;
use std::path::Path;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct GrepTool;

#[async_trait::async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "Search file contents using regex patterns. Supports content, files_with_matches, and count output modes."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in"
                },
                "glob": {
                    "type": "string",
                    "description": "Glob filter for files (e.g., '*.rs')"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Output format (default: files_with_matches)"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Case insensitive search"
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

        let case_insensitive = input
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let regex = {
            let mut builder = regex::RegexBuilder::new(pattern);
            builder.case_insensitive(case_insensitive);
            match builder.build() {
                Ok(r) => r,
                Err(e) => return ToolResult::error(format!("Invalid regex pattern: {e}")),
            }
        };

        let search_path = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let output_mode = input
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files_with_matches");

        let glob_filter = input.get("glob").and_then(|v| v.as_str());

        // Collect files to search
        let files = collect_files(&search_path, glob_filter);

        let mut results: Vec<String> = Vec::new();

        for file_path in &files {
            let content = match fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(_) => continue, // skip binary/unreadable files
            };

            let matches: Vec<(usize, &str)> = content
                .lines()
                .enumerate()
                .filter(|(_, line)| regex.is_match(line))
                .collect();

            if matches.is_empty() {
                continue;
            }

            let path_str = file_path.to_string_lossy();

            match output_mode {
                "files_with_matches" => {
                    results.push(path_str.to_string());
                }
                "count" => {
                    results.push(format!("{path_str}:{}", matches.len()));
                }
                "content" | _ => {
                    for (line_num, line) in &matches {
                        results.push(format!("{path_str}:{}:{}", line_num + 1, line));
                    }
                }
            }
        }

        if results.is_empty() {
            ToolResult::success("No matches found")
        } else {
            ToolResult::success(results.join("\n"))
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

/// Recursively collect files from a path, optionally filtering by glob pattern.
fn collect_files(path: &Path, glob_filter: Option<&str>) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    if path.is_file() {
        files.push(path.to_path_buf());
        return files;
    }

    if !path.is_dir() {
        return files;
    }

    // Use glob if a filter is provided, otherwise walk recursively
    if let Some(filter) = glob_filter {
        let pattern = path.join("**").join(filter);
        if let Ok(entries) = glob::glob(&pattern.to_string_lossy()) {
            for entry in entries.flatten() {
                if entry.is_file() {
                    files.push(entry);
                }
            }
        }
    } else {
        walk_dir(path, &mut files);
    }

    files
}

fn walk_dir(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            files.push(path);
        } else if path.is_dir() {
            // Skip hidden directories and common noise
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with('.')
                && name_str != "node_modules"
                && name_str != "target"
                && name_str != "__pycache__"
            {
                walk_dir(&path, files);
            }
        }
    }
}
