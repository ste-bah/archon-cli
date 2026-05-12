use std::{fs, path::PathBuf};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth};

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
    let source_count =
        count_corpus_files(&cwd.join("docs")) + count_corpus_files(&cwd.join(".archon/kb"));
    let nodes = vec![
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
            count_corpus_files(&cwd.join(".archon/kb")),
        ),
        node(
            "chunks",
            "Chunks",
            "derived",
            "Searchable document spans",
            source_count.saturating_mul(4),
        ),
        node(
            "claims",
            "Claims",
            "evidence",
            "Extracted or cited assertions",
            source_count.saturating_mul(2),
        ),
        node(
            "evidence",
            "Evidence",
            "evidence",
            "Provenance, citations, contradictions",
            source_count,
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
    let edges = vec![
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
    let degraded = nodes.len() as u64 > 3_000 || edges.len() as u64 > 10_000;
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

fn count_corpus_files(path: &PathBuf) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                count_corpus_files(&path)
            } else if is_corpus_file(&path) {
                1
            } else {
                0
            }
        })
        .sum()
}

fn is_corpus_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
        "md" | "txt" | "pdf" | "json" | "jsonl" | "toml" | "yaml" | "yml"
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
}
