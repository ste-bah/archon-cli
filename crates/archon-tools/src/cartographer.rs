pub mod cache;
pub mod index;
pub mod parser;
pub mod summary;

use std::path::Path;
use std::time::SystemTime;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};
use index::CodebaseIndex;
use parser::{language_for_file, parse_file};

/// Default directories to skip during codebase scanning.
const EXCLUDE_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "dist",
    "build",
];

// ---------------------------------------------------------------------------
// CartographerTool
// ---------------------------------------------------------------------------

/// Tool that scans, indexes, queries and summarises a codebase.
pub struct CartographerTool;

#[async_trait::async_trait]
impl Tool for CartographerTool {
    fn name(&self) -> &str {
        "CartographerScan"
    }

    fn description(&self) -> &str {
        "Scan and index a codebase for symbols (structs, classes, functions, etc.). \
         Supports Rust, Python, TypeScript, JavaScript, and Go. \
         Operations: scan (index directory), query (find symbols), \
         summary (token-bounded overview), focus (all symbols in one file)."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["scan", "query", "summary", "focus"],
                    "description": "Operation to perform: scan (index), query (search symbols), summary (overview), focus (single file detail)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to scan (scan) or file to focus on (focus). Defaults to working directory."
                },
                "query": {
                    "type": "string",
                    "description": "Symbol name or substring to search for (query operation)."
                },
                "file": {
                    "type": "string",
                    "description": "Relative file path for focus operation."
                },
                "max_tokens": {
                    "type": "integer",
                    "description": "Token budget for summary output (default: 2000).",
                    "default": 2000
                },
                "force": {
                    "type": "boolean",
                    "description": "Force re-scan ignoring cache (scan operation, default: false).",
                    "default": false
                },
                "exclude_dirs": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Additional directory names to exclude from scanning."
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let operation = match input.get("operation").and_then(|v| v.as_str()) {
            Some(op) => op,
            None => return ToolResult::error("operation is required"),
        };

        match operation {
            "scan" => op_scan(&input, ctx),
            "query" => op_query(&input, ctx),
            "summary" => op_summary(&input, ctx),
            "focus" => op_focus(&input, ctx),
            other => ToolResult::error(format!(
                "Unknown operation '{other}'. Valid: scan, query, summary, focus"
            )),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

fn op_scan(input: &serde_json::Value, ctx: &ToolContext) -> ToolResult {
    let scan_path = input
        .get("path")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ctx.working_dir.clone());

    let force = input
        .get("force")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut extra_excludes: Vec<String> = Vec::new();
    if let Some(arr) = input.get("exclude_dirs").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                extra_excludes.push(s.to_string());
            }
        }
    }

    let mut exclude: Vec<&str> = EXCLUDE_DIRS.to_vec();
    let extra_refs: Vec<&str> = extra_excludes.iter().map(|s| s.as_str()).collect();
    exclude.extend_from_slice(&extra_refs);

    // Attempt to load cache unless forced.
    let cached = if !force {
        cache::load_cache(&scan_path)
    } else {
        None
    };
    let cached_mtimes = cached.as_ref().map(|c| &c.mtimes);

    let mut index = CodebaseIndex::new();

    // Restore cached symbols and mtimes as baseline.
    if let Some(ref c) = cached {
        index.symbols = c.symbols.clone();
        index.mtimes = c.mtimes.clone();
    }

    let mut file_count = 0usize;
    let mut sym_count = 0usize;
    let mut skipped_cached = 0usize;

    scan_directory(
        &scan_path,
        &scan_path,
        &exclude,
        &mut |rel_path: &str, abs_path: &std::path::Path| {
            let language = match language_for_file(rel_path) {
                Some(l) => l,
                None => return,
            };

            let current_mtime = file_mtime(abs_path);

            // Skip if mtime unchanged vs cache.
            if let Some(cached_mtime_map) = cached_mtimes {
                if let Some(&cached_m) = cached_mtime_map.get(rel_path) {
                    if cached_m == current_mtime {
                        skipped_cached += 1;
                        return;
                    }
                }
            }

            let source = match std::fs::read_to_string(abs_path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to read {}: {}", abs_path.display(), e);
                    return;
                }
            };

            let syms = parse_file(rel_path, &source, language);
            sym_count += syms.len();
            index.symbols.insert(rel_path.to_string(), syms);
            index.mtimes.insert(rel_path.to_string(), current_mtime);
            file_count += 1;

            // Extract simple import dependencies via regex scan.
            extract_deps(&source, language, rel_path, &mut index);
        },
    );

    // Persist updated cache.
    let cached_index = cache::CachedIndex {
        symbols: index.symbols.clone(),
        mtimes: index.mtimes.clone(),
    };
    cache::save_cache(&scan_path, &cached_index);

    let total_files = index.symbols.len();
    let total_syms: usize = index.symbols.values().map(|v| v.len()).sum();

    ToolResult::success(format!(
        "Scan complete: {total_files} files indexed, {total_syms} symbols extracted \
         ({file_count} parsed, {skipped_cached} from cache)."
    ))
}

fn op_query(input: &serde_json::Value, ctx: &ToolContext) -> ToolResult {
    let query = match input.get("query").and_then(|v| v.as_str()) {
        Some(q) => q,
        None => return ToolResult::error("query field is required for query operation"),
    };

    let scan_path = input
        .get("path")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ctx.working_dir.clone());

    let index = load_or_build_index(&scan_path, &ctx.working_dir);

    let results = index.find_symbol(query);

    if results.is_empty() {
        return ToolResult::success(format!("No symbols matching '{query}' found."));
    }

    let mut out = format!("Found {} symbol(s) matching '{query}':\n\n", results.len());
    for sym in &results {
        out.push_str(&format!(
            "  [{:?}] {} — {}:{}\n    {}\n",
            sym.kind, sym.name, sym.file, sym.line, sym.signature
        ));
    }

    ToolResult::success(out)
}

fn op_summary(input: &serde_json::Value, ctx: &ToolContext) -> ToolResult {
    let max_tokens = input
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(2000) as usize;

    let scan_path = input
        .get("path")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ctx.working_dir.clone());

    let index = load_or_build_index(&scan_path, &ctx.working_dir);
    let text = summary::generate_summary(&index, max_tokens);

    if text.is_empty() {
        return ToolResult::success("No symbols indexed. Run a scan first.");
    }

    ToolResult::success(text)
}

fn op_focus(input: &serde_json::Value, ctx: &ToolContext) -> ToolResult {
    let file = match input.get("file").and_then(|v| v.as_str()) {
        Some(f) => f,
        None => return ToolResult::error("file field is required for focus operation"),
    };

    let scan_path = input
        .get("path")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ctx.working_dir.clone());

    let index = load_or_build_index(&scan_path, &ctx.working_dir);
    let syms = index.symbols_in_file(file);

    if syms.is_empty() {
        return ToolResult::success(format!(
            "No symbols found for '{file}'. Has the file been scanned?"
        ));
    }

    let mut out = format!("## {file} — {} symbol(s)\n\n", syms.len());
    for sym in syms {
        out.push_str(&format!(
            "  [{:?}] {} (line {})\n  Signature: {}\n\n",
            sym.kind, sym.name, sym.line, sym.signature
        ));
    }

    ToolResult::success(out)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load index from cache or build an empty one (does not re-scan).
fn load_or_build_index(scan_path: &Path, _working_dir: &Path) -> CodebaseIndex {
    let mut index = CodebaseIndex::new();

    if let Some(cached) = cache::load_cache(scan_path) {
        index.symbols = cached.symbols;
        index.mtimes = cached.mtimes;
    }

    index
}

/// Recursively walk `dir`, calling `visitor` with (relative_path, absolute_path)
/// for each file, skipping entries in `exclude`.
fn scan_directory<F>(root: &Path, dir: &Path, exclude: &[&str], visitor: &mut F)
where
    F: FnMut(&str, &Path),
{
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!("Failed to read directory {}: {}", dir.display(), err);
            return;
        }
    };

    for entry in entries.flatten() {
        let abs = entry.path();
        let file_name = match abs.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        if exclude.contains(&file_name.as_str()) {
            continue;
        }

        if abs.is_dir() {
            scan_directory(root, &abs, exclude, visitor);
        } else if abs.is_file() {
            let rel = abs
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| abs.to_string_lossy().into_owned());
            visitor(&rel, &abs);
        }
    }
}

/// Get file mtime as unix timestamp seconds (0 on failure).
fn file_mtime(path: &Path) -> u64 {
    path.metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Simple regex-based import dependency extraction.
///
/// Adds edges to the codebase index's dependency graph.
fn extract_deps(source: &str, language: &str, from_file: &str, index: &mut CodebaseIndex) {
    let pattern = match language {
        "rust" => r#"(?m)^use\s+([\w:]+)"#,
        "python" => r#"(?m)^(?:import|from)\s+([\w.]+)"#,
        "typescript" | "javascript" => r##"(?m)from\s+['"]([^'"]+)['"]"##,
        "go" => r#"(?m)import\s+"([\w./]+)""#,
        _ => return,
    };

    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to compile import regex: {e}");
            return;
        }
    };

    for cap in re.captures_iter(source) {
        if let Some(dep) = cap.get(1) {
            index.add_dep_edge(from_file, dep.as_str());
        }
    }
}

// ---------------------------------------------------------------------------
// Public re-exports so tests can reach sub-modules via cartographer::
// ---------------------------------------------------------------------------
pub use index::{Symbol as CartographerSymbol, SymbolKind as CartographerSymbolKind};
