use std::fs;
use std::path::Path;

use serde_json::json;

use crate::path_guard::resolve_existing_path;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct GrepTool;

const MAX_SEARCH_FILES: usize = 20_000;
const MAX_RESULTS: usize = 2_000;
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

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

        let search_path = match input.get("path").and_then(|v| v.as_str()) {
            Some(path) => match resolve_existing_path(path, ctx) {
                Ok(path) => path,
                Err(err) => return ToolResult::error(err),
            },
            None => match resolve_existing_path(".", ctx) {
                Ok(path) => path,
                Err(err) => return ToolResult::error(err),
            },
        };

        let output_mode = input
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files_with_matches");

        let glob_filter = input.get("glob").and_then(|v| v.as_str());

        // Collect files to search. Use the same bounded walker for glob and
        // non-glob searches so filters like `*` do not traverse target/.
        let (files, file_limit_hit) = match collect_files(&search_path, glob_filter) {
            Ok(result) => result,
            Err(err) => return ToolResult::error(err),
        };

        let mut results: Vec<String> = Vec::new();
        let mut result_limit_hit = false;
        let mut skipped_large = 0usize;

        for file_path in &files {
            if results.len() >= MAX_RESULTS {
                result_limit_hit = true;
                break;
            }
            if file_too_large(file_path) {
                skipped_large += 1;
                continue;
            }
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
                _ => {
                    for (line_num, line) in &matches {
                        if results.len() >= MAX_RESULTS {
                            result_limit_hit = true;
                            break;
                        }
                        results.push(format!("{path_str}:{}:{}", line_num + 1, line));
                    }
                }
            }
        }

        if results.is_empty() {
            let mut message = "No matches found".to_string();
            append_limits(
                &mut message,
                file_limit_hit,
                result_limit_hit,
                skipped_large,
            );
            ToolResult::success(message)
        } else {
            let mut output = results.join("\n");
            append_limits(&mut output, file_limit_hit, result_limit_hit, skipped_large);
            ToolResult::success(output)
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

/// Recursively collect files from a path, optionally filtering by glob pattern.
fn collect_files(
    path: &Path,
    glob_filter: Option<&str>,
) -> Result<(Vec<std::path::PathBuf>, bool), String> {
    let mut files = Vec::new();

    if path.is_file() {
        files.push(path.to_path_buf());
        return Ok((files, false));
    }

    if !path.is_dir() {
        return Ok((files, false));
    }

    let matcher = match glob_filter {
        Some(filter) => Some(glob::Pattern::new(filter).map_err(|e| format!("{e}"))?),
        None => None,
    };
    let limit_hit = walk_dir(path, path, matcher.as_ref(), &mut files);

    Ok((files, limit_hit))
}

fn walk_dir(
    base: &Path,
    dir: &Path,
    matcher: Option<&glob::Pattern>,
    files: &mut Vec<std::path::PathBuf>,
) -> bool {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };

    for entry in entries.flatten() {
        if files.len() >= MAX_SEARCH_FILES {
            return true;
        }
        let path = entry.path();
        if path.is_file() {
            if matches_glob(base, &path, matcher) {
                files.push(path);
            }
        } else if path.is_dir() {
            if !skip_dir(&entry.file_name().to_string_lossy())
                && walk_dir(base, &path, matcher, files)
            {
                return true;
            }
        }
    }
    false
}

fn skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules"
                | "target"
                | "__pycache__"
                | "dist"
                | "build"
                | "coverage"
                | ".next"
                | ".turbo"
                | ".venv"
        )
}

fn matches_glob(base: &Path, path: &Path, matcher: Option<&glob::Pattern>) -> bool {
    let Some(pattern) = matcher else {
        return true;
    };
    let rel = path.strip_prefix(base).unwrap_or(path);
    pattern.matches_path(rel)
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| pattern.matches(name))
}

fn file_too_large(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.len() > MAX_FILE_BYTES)
        .unwrap_or(false)
}

fn append_limits(
    output: &mut String,
    file_limit_hit: bool,
    result_limit_hit: bool,
    skipped_large: usize,
) {
    let mut notes = Vec::new();
    if file_limit_hit {
        notes.push(format!("file scan capped at {MAX_SEARCH_FILES} files"));
    }
    if result_limit_hit {
        notes.push(format!("results capped at {MAX_RESULTS} matches"));
    }
    if skipped_large > 0 {
        notes.push(format!(
            "skipped {skipped_large} file(s) over {MAX_FILE_BYTES} bytes"
        ));
    }
    if !notes.is_empty() {
        output.push_str("\n\n[grep truncated: ");
        output.push_str(&notes.join("; "));
        output.push(']');
    }
}
