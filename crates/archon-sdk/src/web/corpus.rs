use std::{
    fs,
    path::{Path, PathBuf},
};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth, inspect::PathProbe};

const SOURCE_LIMIT: usize = 200;
const PREVIEW_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CorpusSource {
    pub label: String,
    pub path: String,
    pub kind: String,
    pub bytes: u64,
    pub excerpt: Option<String>,
    pub score: f64,
    pub match_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CorpusSummary {
    pub roots: Vec<PathProbe>,
    pub sources: Vec<CorpusSource>,
    pub total_sources: u64,
    pub degraded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CorpusSearchQuery {
    pub query: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CorpusSearchResponse {
    pub query: String,
    pub kind: String,
    pub total_matches: u64,
    pub degraded: bool,
    pub ranking_mode: String,
    pub chunk_matches: Vec<CorpusChunkHit>,
    pub results: Vec<CorpusSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CorpusChunkHit {
    pub source_label: String,
    pub source_path: String,
    pub chunk_label: String,
    pub line_start: u64,
    pub score: f64,
    pub excerpt: String,
    pub embedding_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CorpusPreviewQuery {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CorpusSourcePreview {
    pub source: CorpusSource,
    pub content: String,
    pub line_count: u64,
    pub truncated: bool,
    pub preview_available: bool,
    pub policy_reason: String,
}

pub(crate) async fn summary_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    authed_json(&state, &headers, corpus_summary())
}

pub(crate) async fn search_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CorpusSearchQuery>,
) -> Response {
    authed_json(&state, &headers, corpus_search(query))
}

pub(crate) async fn preview_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CorpusPreviewQuery>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    match corpus_preview(query) {
        Ok(preview) => (StatusCode::OK, Json(preview)).into_response(),
        Err(reason) => (StatusCode::BAD_REQUEST, reason).into_response(),
    }
}

fn authed_json<T: Serialize>(state: &AppState, headers: &HeaderMap, value: T) -> Response {
    if let Err(resp) = check_auth(state, headers) {
        return resp;
    }
    (StatusCode::OK, Json(value)).into_response()
}

fn corpus_summary() -> CorpusSummary {
    let roots = corpus_roots()
        .into_iter()
        .map(|(label, path)| probe(label, path))
        .collect();
    let sources = collect_limited_sources(SOURCE_LIMIT);
    CorpusSummary {
        roots,
        total_sources: sources.len() as u64,
        degraded: sources.len() >= SOURCE_LIMIT,
        sources,
    }
}

fn corpus_search(query: CorpusSearchQuery) -> CorpusSearchResponse {
    let needle = query.query.unwrap_or_default().to_lowercase();
    let kind = query.kind.unwrap_or_default().to_lowercase();
    let limit = query.limit.unwrap_or(50).min(SOURCE_LIMIT);
    let mut scored_sources = Vec::new();
    let mut chunk_matches = Vec::new();
    for source in collect_limited_sources(SOURCE_LIMIT) {
        if !kind.is_empty() && source.kind.to_lowercase() != kind {
            continue;
        }
        let mut source = with_excerpt(source, &needle);
        let source_score = source_score(&source, &needle);
        let mut hits = chunk_hits_for_source(&source, &needle, 4);
        let chunk_score = hits.first().map(|hit| hit.score).unwrap_or(0.0);
        source.score = source_score.max(chunk_score);
        source.match_kind = if chunk_score > source_score {
            "chunk".into()
        } else {
            "source".into()
        };
        if needle.is_empty() || source.score > 0.0 {
            chunk_matches.append(&mut hits);
            scored_sources.push(source);
        }
    }
    scored_sources.sort_by(|a, b| score_order(b.score, a.score));
    chunk_matches.sort_by(|a, b| score_order(b.score, a.score));
    let total_matches = scored_sources.len() as u64;
    let results = scored_sources.into_iter().take(limit).collect();
    CorpusSearchResponse {
        query: needle,
        kind,
        total_matches,
        degraded: total_matches as usize > limit,
        ranking_mode: "chunk_lexical_with_embedding_hints".into(),
        chunk_matches: chunk_matches.into_iter().take(12).collect(),
        results,
    }
}

fn corpus_preview(query: CorpusPreviewQuery) -> Result<CorpusSourcePreview, String> {
    let path = PathBuf::from(query.path);
    if !is_inside_corpus_root(&path) {
        return Err("path is outside configured corpus roots".into());
    }
    let Some(source) = source_from_path(&path) else {
        return Err("path is not a supported corpus file".into());
    };
    if !is_text_preview(&source.kind) {
        return Ok(preview_unavailable(
            source,
            "binary preview is not available yet",
        ));
    }
    let bytes = fs::read(&path).map_err(|err| format!("failed to read corpus source: {err}"))?;
    let truncated = bytes.len() > PREVIEW_LIMIT;
    let content = String::from_utf8_lossy(&bytes[..bytes.len().min(PREVIEW_LIMIT)]).to_string();
    let line_count = content.lines().count() as u64;
    Ok(CorpusSourcePreview {
        source,
        content,
        line_count,
        truncated,
        preview_available: true,
        policy_reason: "read-only preview under configured corpus root".into(),
    })
}

fn collect_limited_sources(limit: usize) -> Vec<CorpusSource> {
    let mut sources = Vec::new();
    for (_, root) in corpus_roots() {
        collect_sources(&root, 0, limit, &mut sources);
        if sources.len() >= limit {
            break;
        }
    }
    sources
}

fn collect_sources(root: &Path, depth: usize, limit: usize, out: &mut Vec<CorpusSource>) {
    if depth > 3 || out.len() >= limit {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || matches!(name.as_str(), "target" | "node_modules") {
            continue;
        }
        if path.is_dir() {
            collect_sources(&path, depth + 1, limit, out);
        } else if let Some(source) = source_from_path(&path) {
            out.push(source);
        }
        if out.len() >= limit {
            break;
        }
    }
}

fn source_from_path(path: &Path) -> Option<CorpusSource> {
    if !is_corpus_file(path) {
        return None;
    }
    let kind = path.extension().and_then(|e| e.to_str()).unwrap_or("file");
    Some(CorpusSource {
        label: path.file_name()?.to_string_lossy().to_string(),
        path: path.to_string_lossy().to_string(),
        kind: kind.into(),
        bytes: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        excerpt: None,
        score: 0.0,
        match_kind: "source".into(),
    })
}

fn with_excerpt(mut source: CorpusSource, needle: &str) -> CorpusSource {
    source.excerpt = source_excerpt(&source.path).map(|text| excerpt_around(&text, needle));
    source
}

fn source_excerpt(path: &str) -> Option<String> {
    let source = source_from_path(Path::new(path))?;
    if !is_text_preview(&source.kind) {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes[..bytes.len().min(8 * 1024)]).to_string())
}

fn excerpt_around(text: &str, needle: &str) -> String {
    let trimmed = text.trim();
    if needle.is_empty() {
        return trimmed.chars().take(240).collect();
    }
    let lower = trimmed.to_lowercase();
    let start = lower.find(needle).unwrap_or(0).saturating_sub(80);
    trimmed.chars().skip(start).take(260).collect()
}

fn source_score(source: &CorpusSource, needle: &str) -> f64 {
    if needle.is_empty() {
        return 1.0;
    }
    let label = source.label.to_lowercase();
    let path = source.path.to_lowercase();
    let excerpt = source.excerpt.as_deref().unwrap_or("").to_lowercase();
    weighted_term_score(&label, needle) * 4.0
        + weighted_term_score(&path, needle) * 2.0
        + weighted_term_score(&excerpt, needle)
}

fn chunk_hits_for_source(source: &CorpusSource, needle: &str, limit: usize) -> Vec<CorpusChunkHit> {
    if needle.is_empty() || !is_text_preview(&source.kind) {
        return Vec::new();
    }
    let Ok(bytes) = fs::read(&source.path) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(PREVIEW_LIMIT)]).to_string();
    ranked_chunks(&text, source, needle)
        .into_iter()
        .take(limit)
        .collect()
}

fn ranked_chunks(text: &str, source: &CorpusSource, needle: &str) -> Vec<CorpusChunkHit> {
    let mut hits = Vec::new();
    let mut line_start = 1;
    for (index, chunk) in text.split("\n\n").enumerate() {
        let score = weighted_term_score(&chunk.to_lowercase(), needle);
        if score > 0.0 {
            hits.push(CorpusChunkHit {
                source_label: source.label.clone(),
                source_path: source.path.clone(),
                chunk_label: format!("chunk {}", index + 1),
                line_start,
                score,
                excerpt: excerpt_around(chunk, needle),
                embedding_status: embedding_status_for(source),
            });
        }
        line_start += chunk.lines().count() as u64 + 1;
    }
    hits.sort_by(|a, b| score_order(b.score, a.score));
    hits
}

fn weighted_term_score(haystack: &str, needle: &str) -> f64 {
    let exact = haystack.matches(needle).count() as f64;
    let term_score: f64 = needle
        .split_whitespace()
        .filter(|term| term.len() > 1)
        .map(|term| haystack.matches(term).count() as f64 * 0.35)
        .sum();
    exact + term_score
}

fn embedding_status_for(source: &CorpusSource) -> String {
    if source.path.contains(".archon/docs") || source.path.contains(".archon/kb") {
        "indexed-corpus".into()
    } else {
        "filesystem".into()
    }
}

fn score_order(left: f64, right: f64) -> std::cmp::Ordering {
    left.partial_cmp(&right)
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn preview_unavailable(source: CorpusSource, reason: &str) -> CorpusSourcePreview {
    CorpusSourcePreview {
        source,
        content: String::new(),
        line_count: 0,
        truncated: false,
        preview_available: false,
        policy_reason: reason.into(),
    }
}

fn is_inside_corpus_root(path: &Path) -> bool {
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    corpus_roots()
        .into_iter()
        .filter_map(|(_, root)| root.canonicalize().ok())
        .any(|root| path.starts_with(root))
}

fn corpus_roots() -> Vec<(String, PathBuf)> {
    let cwd = cwd();
    vec![
        ("repo docs".into(), cwd.join("docs")),
        ("local kb".into(), cwd.join(".archon/kb")),
        ("local docs store".into(), cwd.join(".archon/docs")),
        ("home kb".into(), home_archon().join("kb")),
    ]
}

fn is_corpus_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
        "md" | "txt" | "pdf" | "json" | "jsonl" | "toml" | "yaml" | "yml"
    )
}

fn is_text_preview(kind: &str) -> bool {
    matches!(
        kind,
        "md" | "txt" | "json" | "jsonl" | "toml" | "yaml" | "yml"
    )
}

fn probe(label: impl Into<String>, path: PathBuf) -> PathProbe {
    let (files, bytes) = dir_stats(&path, 0);
    PathProbe {
        label: label.into(),
        path: path.to_string_lossy().to_string(),
        exists: path.exists(),
        files,
        bytes,
    }
}

fn dir_stats(path: &Path, depth: usize) -> (u64, u64) {
    if depth > 3 {
        return (0, 0);
    }
    let Ok(metadata) = fs::metadata(path) else {
        return (0, 0);
    };
    if metadata.is_file() {
        return (1, metadata.len());
    }
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| dir_stats(&entry.path(), depth + 1))
        .fold((0, 0), |(files, bytes), (child_files, child_bytes)| {
            (files + child_files, bytes + child_bytes)
        })
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn home_archon() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".archon")
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(CorpusSource::decl(&cfg)),
        exported(CorpusSummary::decl(&cfg)),
        exported(CorpusSearchQuery::decl(&cfg)),
        exported(CorpusSearchResponse::decl(&cfg)),
        exported(CorpusChunkHit::decl(&cfg)),
        exported(CorpusPreviewQuery::decl(&cfg)),
        exported(CorpusSourcePreview::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_rejects_paths_outside_corpus_roots() {
        let result = corpus_preview(CorpusPreviewQuery {
            path: "/etc/passwd".into(),
        });
        assert!(result.is_err());
    }
}
