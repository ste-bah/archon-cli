use std::{
    fs,
    path::{Path, PathBuf},
};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth};

const GRAPH_SOURCE_LIMIT: usize = 200;
const SOURCE_NODE_LIMIT: usize = 48;
const TEXT_SCAN_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EvidenceGraphNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub detail: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EvidenceGraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EvidenceGraphSummary {
    pub node_budget: u64,
    pub edge_budget: u64,
    pub source_count: u64,
    pub relation_count: u64,
    pub degraded: bool,
    pub nodes: Vec<EvidenceGraphNode>,
    pub edges: Vec<EvidenceGraphEdge>,
}

pub(crate) async fn graph_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    (StatusCode::OK, Json(evidence_graph())).into_response()
}

fn evidence_graph() -> EvidenceGraphSummary {
    let cwd = cwd();
    let home = home_archon();
    let sources = collect_graph_sources(GRAPH_SOURCE_LIMIT);
    let source_count = sources.len() as u64;
    let chunk_count: u64 = sources.iter().map(|source| source.stats.chunks).sum();
    let claim_count: u64 = sources.iter().map(|source| source.stats.claims).sum();
    let evidence_count: u64 = sources
        .iter()
        .map(|source| source.stats.evidence_refs)
        .sum::<u64>()
        + count_entries(&cwd.join(".archon/evidence"))
        + count_entries(&home.join("evidence"))
        + count_entries(&home.join("provenance"));

    let mut nodes = vec![
        node(
            "docs",
            "Docs",
            "source",
            "Filesystem documents and PDFs",
            source_count,
        ),
        node(
            "kb",
            "Knowledge base",
            "source",
            "Compiled /kb material",
            sources
                .iter()
                .filter(|source| source.root_id == "kb")
                .count() as u64,
        ),
        node(
            "chunks",
            "Chunks",
            "derived",
            "Text spans counted from corpus files",
            chunk_count,
        ),
        node(
            "claims",
            "Claims",
            "evidence",
            "Claim markers counted from corpus text",
            claim_count,
        ),
        node(
            "evidence",
            "Evidence",
            "evidence",
            "Citation, source, provenance, and evidence markers",
            evidence_count,
        ),
        node(
            "memory",
            "Memory",
            "learning",
            "Persistent graph and recall rows",
            count_entries(&home.join("memory")),
        ),
        node(
            "learning",
            "LearningEvents",
            "learning",
            "Governed behavioural learning rows",
            count_entries(&home.join("learning")),
        ),
        node(
            "reasoning",
            "Reasoning quality",
            "reasoning",
            "Claim and correction events",
            count_entries(&home.join("reasoning-quality")),
        ),
        node(
            "world",
            "World model",
            "model",
            "Trace rows, candidates, advisor events",
            count_entries(&home.join("world-model")),
        ),
        node(
            "sessions",
            "Sessions",
            "runtime",
            "Agent turns and activity ledgers",
            count_entries(&home.join("sessions")),
        ),
        node(
            "pipelines",
            "Pipelines",
            "runtime",
            "Pipeline stages, agents, and runs",
            count_entries(&home.join("pipelines")),
        ),
        node(
            "artifacts",
            "Artifacts",
            "output",
            "Reports, bundles, and generated files",
            count_entries(&cwd.join(".archon/artifacts")),
        ),
    ];

    let mut edges = vec![
        edge("docs_chunks", "docs", "chunks", "ingested as"),
        edge("kb_chunks", "kb", "chunks", "compiled into"),
        edge("chunks_claims", "chunks", "claims", "support"),
        edge("claims_evidence", "claims", "evidence", "grounded by"),
        edge("evidence_memory", "evidence", "memory", "stored as"),
        edge("memory_learning", "memory", "learning", "feeds"),
        edge("sessions_reasoning", "sessions", "reasoning", "emits"),
        edge("reasoning_learning", "reasoning", "learning", "bridges to"),
        edge("reasoning_world", "reasoning", "world", "adds trace signal"),
        edge("sessions_pipelines", "sessions", "pipelines", "runs"),
        edge("pipelines_artifacts", "pipelines", "artifacts", "writes"),
        edge("world_pipelines", "world", "pipelines", "advises"),
    ];

    for source in sources.iter().take(SOURCE_NODE_LIMIT) {
        nodes.push(node(
            &source.id,
            &source.label,
            "source_file",
            &source.path,
            source.bytes,
        ));
        edges.push(edge(
            &format!("{}_{}", source.root_id, source.id),
            &source.root_id,
            &source.id,
            "contains",
        ));
        if source.stats.chunks > 0 {
            edges.push(edge(
                &format!("{}_chunks", source.id),
                &source.id,
                "chunks",
                "has text spans",
            ));
        }
        if source.stats.claims > 0 {
            edges.push(edge(
                &format!("{}_claims", source.id),
                &source.id,
                "claims",
                "mentions claim markers",
            ));
        }
        if source.stats.evidence_refs > 0 {
            edges.push(edge(
                &format!("{}_evidence", source.id),
                &source.id,
                "evidence",
                "mentions evidence markers",
            ));
        }
    }

    let degraded = sources.len() >= GRAPH_SOURCE_LIMIT
        || nodes.len() as u64 > 3_000
        || edges.len() as u64 > 10_000;
    EvidenceGraphSummary {
        node_budget: 3_000,
        edge_budget: 10_000,
        source_count,
        relation_count: edges.len() as u64,
        degraded,
        nodes,
        edges,
    }
}

fn node(id: &str, label: &str, kind: &str, detail: &str, count: u64) -> EvidenceGraphNode {
    EvidenceGraphNode {
        id: id.into(),
        label: label.into(),
        kind: kind.into(),
        detail: detail.into(),
        count,
    }
}

fn edge(id: &str, source: &str, target: &str, label: &str) -> EvidenceGraphEdge {
    EvidenceGraphEdge {
        id: id.into(),
        source: source.into(),
        target: target.into(),
        label: label.into(),
    }
}

fn count_entries(path: &PathBuf) -> u64 {
    fs::read_dir(path)
        .map(|entries| entries.flatten().count() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
struct GraphSource {
    id: String,
    label: String,
    path: String,
    root_id: String,
    bytes: u64,
    stats: TextStats,
}

#[derive(Debug, Clone, Copy, Default)]
struct TextStats {
    chunks: u64,
    claims: u64,
    evidence_refs: u64,
}

fn collect_graph_sources(limit: usize) -> Vec<GraphSource> {
    let mut sources = Vec::new();
    for (root_id, root) in corpus_roots() {
        collect_sources(&root, &root_id, 0, limit, &mut sources);
        if sources.len() >= limit {
            break;
        }
    }
    sources
}

fn collect_sources(
    root: &Path,
    root_id: &str,
    depth: usize,
    limit: usize,
    out: &mut Vec<GraphSource>,
) {
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
            collect_sources(&path, root_id, depth + 1, limit, out);
        } else if is_corpus_file(&path) {
            out.push(graph_source(root_id, out.len(), &path));
        }
        if out.len() >= limit {
            break;
        }
    }
}

fn graph_source(root_id: &str, index: usize, path: &Path) -> GraphSource {
    let label = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "source".to_string());
    GraphSource {
        id: format!("source_{index}"),
        label,
        path: path.to_string_lossy().to_string(),
        root_id: root_id.to_string(),
        bytes: fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
        stats: text_stats(path),
    }
}

fn text_stats(path: &Path) -> TextStats {
    if !is_text_file(path) {
        return TextStats::default();
    }
    let Ok(bytes) = fs::read(path) else {
        return TextStats::default();
    };
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(TEXT_SCAN_LIMIT)]);
    let chunks = text
        .split("\n\n")
        .filter(|chunk| !chunk.trim().is_empty())
        .count() as u64;
    let claims = text
        .lines()
        .filter(|line| contains_any(line, &["claim", "assert", "finding", "verdict"]))
        .count() as u64;
    let evidence_refs = text
        .lines()
        .filter(|line| {
            contains_any(
                line,
                &[
                    "evidence",
                    "source",
                    "citation",
                    "provenance",
                    "http://",
                    "https://",
                    "[^",
                ],
            )
        })
        .count() as u64;
    TextStats {
        chunks,
        claims,
        evidence_refs,
    }
}

fn contains_any(line: &str, needles: &[&str]) -> bool {
    let line = line.to_ascii_lowercase();
    needles.iter().any(|needle| line.contains(needle))
}

fn corpus_roots() -> Vec<(String, PathBuf)> {
    let cwd = cwd();
    vec![
        ("docs".into(), cwd.join("docs")),
        ("kb".into(), cwd.join(".archon/kb")),
        ("docs".into(), cwd.join(".archon/docs")),
        ("kb".into(), home_archon().join("kb")),
    ]
}

fn is_corpus_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
        "md" | "txt" | "pdf" | "json" | "jsonl" | "toml" | "yaml" | "yml"
    )
}

fn is_text_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
        "md" | "txt" | "json" | "jsonl" | "toml" | "yaml" | "yml"
    )
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
        exported(EvidenceGraphNode::decl(&cfg)),
        exported(EvidenceGraphEdge::decl(&cfg)),
        exported(EvidenceGraphSummary::decl(&cfg)),
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
    fn graph_contains_reasoning_to_world_edge() {
        let graph = evidence_graph();
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.source == "reasoning" && edge.target == "world")
        );
    }

    #[test]
    fn text_stats_are_derived_from_file_content() {
        let path =
            std::env::temp_dir().join(format!("archon-evidence-{}.md", uuid::Uuid::new_v4()));
        fs::write(
            &path,
            "First claim with evidence.\n\nSecond finding cites https://example.test/source",
        )
        .expect("write evidence fixture");

        let stats = text_stats(&path);
        let _ = fs::remove_file(path);

        assert_eq!(stats.chunks, 2);
        assert_eq!(stats.claims, 2);
        assert_eq!(stats.evidence_refs, 2);
    }
}
