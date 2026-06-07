use std::path::{Path, PathBuf};

use archon_docs::{answer, retrieval};
use cozo::DbInstance;

use crate::tool::{ToolContext, ToolResult};

const DOCS_DB_ENV: &str = "ARCHON_DOCS_DB_PATH";
const EVIDENCE_DB_ENV: &str = "ARCHON_EVIDENCE_DB_PATH";

pub(crate) async fn run_search(args: Vec<String>, ctx: &ToolContext) -> ToolResult {
    let parsed = match SearchArgs::parse(args) {
        Ok(args) => args,
        Err(error) => return ToolResult::error(error),
    };
    let db = match open_docs_db(ctx) {
        Ok(db) => db,
        Err(error) => return ToolResult::error(error),
    };
    let mode = match retrieval::SearchMode::parse(&parsed.mode) {
        Ok(mode) => mode,
        Err(error) => return ToolResult::error(error.to_string()),
    };
    let policy = load_policy(ctx);
    match retrieval::search_with_policy(&db, &parsed.query, 10, mode, &policy) {
        Ok(results) => ToolResult::success(format_search_results(&db, &results, parsed.debug)),
        Err(error) => ToolResult::error(format_search_error(error)),
    }
}

pub(crate) async fn run_answer(args: Vec<String>, ctx: &ToolContext) -> ToolResult {
    let query = match answer_query(args) {
        Ok(query) => query,
        Err(error) => return ToolResult::error(error),
    };
    let db = match open_docs_db(ctx) {
        Ok(db) => db,
        Err(error) => return ToolResult::error(error),
    };
    match answer::answer(&db, &query, 5) {
        Ok(ans) => ToolResult::success(format_answer(&db, ans)),
        Err(error) => ToolResult::error(format_search_error(error)),
    }
}

struct SearchArgs {
    query: String,
    mode: String,
    debug: bool,
}

impl SearchArgs {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut iter = args.into_iter();
        expect_arg(&mut iter, "docs")?;
        expect_arg(&mut iter, "search")?;
        let query = iter
            .next()
            .ok_or_else(|| "docs search query is required".to_string())?;
        let mut mode = "hybrid".to_string();
        let mut debug = false;
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--mode" => {
                    mode = iter
                        .next()
                        .ok_or_else(|| "--mode requires a value".to_string())?;
                }
                "--debug" => debug = true,
                other => return Err(format!("unsupported docs search arg '{other}'")),
            }
        }
        Ok(Self { query, mode, debug })
    }
}

fn answer_query(args: Vec<String>) -> Result<String, String> {
    let mut iter = args.into_iter();
    expect_arg(&mut iter, "docs")?;
    expect_arg(&mut iter, "answer")?;
    iter.next()
        .ok_or_else(|| "docs answer query is required".to_string())
}

fn expect_arg(iter: &mut impl Iterator<Item = String>, expected: &str) -> Result<(), String> {
    match iter.next().as_deref() {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(format!("expected '{expected}', got '{actual}'")),
        None => Err(format!("expected '{expected}'")),
    }
}

fn open_docs_db(ctx: &ToolContext) -> Result<DbInstance, String> {
    let cwd = working_dir(ctx);
    let db_path = docs_db_path(&cwd);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create docs DB dir {}: {error}", parent.display()))?;
    }
    archon_docs::configure_cozo_write_lock_for_db(&db_path);
    let path = db_path.to_string_lossy().to_string();
    let db = DbInstance::new("sqlite", &path, "")
        .map_err(|error| format!("open document store at {path}: {error}"))?;
    archon_docs::schema::ensure_doc_schema(&db)
        .map_err(|error| format!("ensure document schema: {error}"))?;
    Ok(db)
}

fn working_dir(ctx: &ToolContext) -> PathBuf {
    if ctx.working_dir.as_os_str().is_empty() {
        return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    }
    ctx.working_dir.clone()
}

fn docs_db_path(cwd: &Path) -> PathBuf {
    [DOCS_DB_ENV, EVIDENCE_DB_ENV]
        .into_iter()
        .find_map(|key| std::env::var_os(key).filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join(".archon").join("archon-data.db"))
}

fn load_policy(ctx: &ToolContext) -> archon_policy::EffectivePolicy {
    archon_policy::load_effective_policy(&working_dir(ctx)).unwrap_or_default()
}

fn format_search_results(
    db: &DbInstance,
    results: &retrieval::SearchResults,
    debug: bool,
) -> String {
    if results.results.is_empty() && results.total_chunks == 0 {
        return "No documents indexed. Use 'archon docs ingest <path>' first.".into();
    }
    if results.results.is_empty() {
        return format!(
            "No results found. {} chunks stored, {} chunks indexed, but none matched your query.",
            results.total_chunks, results.total_indexed_chunks
        );
    }
    let mut out = format!(
        "Found {} result(s) ({} chunks indexed, mode={}):\n",
        results.results.len(),
        results.total_indexed_chunks,
        results.mode.as_str()
    );
    if debug {
        match results.query_embedding_norm {
            Some(norm) => out.push_str(&format!("\nQuery embedding norm: {norm:.6}\n")),
            None => out.push_str("\nQuery embedding norm: n/a\n"),
        }
        out.push_str("Top-k raw scores and citation chains:\n");
    }
    for (i, result) in results.results.iter().enumerate() {
        out.push_str(&format!(
            "  {}. {}  pages {}-{}  score={:.3}\n",
            i + 1,
            result.chunk_id,
            result.page_start,
            result.page_end,
            result.score
        ));
        if debug {
            push_debug_result(db, &mut out, result);
        }
    }
    for warning in &results.warnings {
        out.push_str(&format!("Warning: {warning}\n"));
    }
    out.trim_end().to_string()
}

fn push_debug_result(db: &DbInstance, out: &mut String, result: &retrieval::SearchResult) {
    out.push_str(&format!("     document: {}\n", result.document_id));
    out.push_str(&format!(
        "     raw distance:        {:.4}\n",
        result.distance
    ));
    out.push_str(&format!(
        "     raw exact score:     {:.4}\n",
        result.exact_score
    ));
    out.push_str(&format!(
        "     raw semantic score:  {:.4}\n",
        result.semantic_score
    ));
    if let Ok(Some(doc)) = archon_docs::store::get_doc_source(db, &result.document_id) {
        out.push_str(&format!("     source: {}\n", doc.source_path));
    }
    out.push_str(&format!("     content:  {}\n", preview(&result.content)));
}

fn format_answer(db: &DbInstance, ans: answer::Answer) -> String {
    let edge_count = answer::persist_answer_provenance(db, &ans).unwrap_or(0);
    let mut out = format!("Answer ID: {}\n\n{}\n", ans.answer_id, ans.text);
    if !ans.citations.is_empty() {
        out.push_str(&format!("\nCitations ({edge_count} provenance edge(s)):\n"));
        for (i, citation) in ans.citations.iter().enumerate() {
            out.push_str(&format!(
                "  [{}] {}  pages {}-{}  doc={}\n",
                i + 1,
                citation.chunk_id,
                citation.page_start,
                citation.page_end,
                citation.document_id
            ));
        }
    }
    out.trim_end().to_string()
}

fn preview(content: &str) -> String {
    const MAX: usize = 120;
    let mut chars = content.chars();
    let preview: String = chars.by_ref().take(MAX).collect();
    if chars.next().is_none() {
        return content.to_string();
    }
    format!("{preview}...")
}

fn format_search_error(error: archon_docs::errors::DocsError) -> String {
    match error {
        archon_docs::errors::DocsError::Embedding { message }
        | archon_docs::errors::DocsError::ModelNotConfigured { message } => message,
        other => other.to_string(),
    }
}
