use std::path::Path;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::inspect::PathProbe;
use super::{AppState, check_auth};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CognitiveWebSummary {
    pub store: PathProbe,
    pub store_present: bool,
    pub situation_count: u64,
    pub tool_decision_count: u64,
    pub executive_decision_count: u64,
    pub reflection_count: u64,
    pub proposal_count: u64,
    pub apply_result_count: u64,
    pub self_model_fact_count: u64,
    pub daemon: CognitiveDaemonPreview,
    pub latest_tick: Option<CognitiveTickPreview>,
    pub decisions: Vec<CognitiveRowPreview>,
    pub reflections: Vec<CognitiveRowPreview>,
    pub proposals: Vec<CognitiveRowPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CognitiveRowPreview {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CognitiveTickPreview {
    pub tick_id: String,
    pub proposals_evaluated: u64,
    pub proposals_auto_applied: u64,
    pub proposals_denied: u64,
    pub error_count: u64,
    pub duration_ms: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CognitiveDaemonPreview {
    pub running: bool,
    pub stale: bool,
    pub stop_requested: bool,
    pub ticks_run: u64,
    pub pid: Option<u32>,
    pub last_heartbeat_at: Option<String>,
}

pub(crate) async fn summary_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    (StatusCode::OK, Json(summary_for_root(&state.paths.cwd))).into_response()
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        CognitiveWebSummary::decl(&cfg),
        CognitiveRowPreview::decl(&cfg),
        CognitiveTickPreview::decl(&cfg),
        CognitiveDaemonPreview::decl(&cfg),
    ]
    .into_iter()
    .map(|decl| format!("export {decl}"))
    .collect::<Vec<_>>()
    .join("\n")
}

fn summary_for_root(cwd: &Path) -> CognitiveWebSummary {
    let root = cwd.join(".archon").join("cognitive");
    let db_path = root.join("cognitive.db");
    let store = path_probe("cognitive store", &root);
    if !db_path.exists() {
        return empty_summary(store);
    }
    let Ok(store_handle) = archon_cognitive::PersistentCognitiveStore::open(&root) else {
        return empty_summary(store);
    };
    let Ok(inspection) = archon_cognitive::CognitiveInspection::new(store_handle.db(), &root)
    else {
        return empty_summary(store);
    };
    let Ok(status) = inspection.status() else {
        return empty_summary(store);
    };
    CognitiveWebSummary {
        store,
        store_present: true,
        situation_count: status.situation_count as u64,
        tool_decision_count: status.tool_decision_count as u64,
        executive_decision_count: status.executive_decision_count as u64,
        reflection_count: status.reflection_count as u64,
        proposal_count: status.proposal_count as u64,
        apply_result_count: status.apply_result_count as u64,
        self_model_fact_count: status.self_model_fact_count as u64,
        daemon: daemon_preview(&root),
        latest_tick: status.latest_tick.map(|tick| CognitiveTickPreview {
            tick_id: tick.tick_id,
            proposals_evaluated: tick.proposals_evaluated,
            proposals_auto_applied: tick.proposals_auto_applied,
            proposals_denied: tick.proposals_denied,
            error_count: tick.error_count as u64,
            duration_ms: tick.duration_ms,
            created_at: tick.created_at.to_rfc3339(),
        }),
        decisions: status
            .recent_decisions
            .into_iter()
            .map(|decision| CognitiveRowPreview {
                id: decision.decision_id,
                label: "decision".into(),
                status: decision.selected_candidate_id,
                detail: decision.user_visible_summary,
                created_at: decision.created_at.to_rfc3339(),
            })
            .collect(),
        reflections: status
            .recent_reflections
            .into_iter()
            .map(|reflection| CognitiveRowPreview {
                id: reflection.reflection_id,
                label: reflection.situation_kind,
                status: reflection.outcome,
                detail: reflection.lesson,
                created_at: reflection.created_at.to_rfc3339(),
            })
            .collect(),
        proposals: status
            .pending_proposals
            .into_iter()
            .map(|proposal| CognitiveRowPreview {
                id: proposal.proposal_id,
                label: proposal.manifest_kind,
                status: proposal
                    .latest_result
                    .unwrap_or_else(|| proposal.risk_level.clone()),
                detail: format!(
                    "{} evidence - {}",
                    proposal.evidence_count, proposal.lesson_tag
                ),
                created_at: proposal.created_at.to_rfc3339(),
            })
            .collect(),
    }
}

fn empty_summary(store: PathProbe) -> CognitiveWebSummary {
    CognitiveWebSummary {
        store,
        store_present: false,
        situation_count: 0,
        tool_decision_count: 0,
        executive_decision_count: 0,
        reflection_count: 0,
        proposal_count: 0,
        apply_result_count: 0,
        self_model_fact_count: 0,
        daemon: daemon_preview_for_missing_store(),
        latest_tick: None,
        decisions: Vec::new(),
        reflections: Vec::new(),
        proposals: Vec::new(),
    }
}

fn daemon_preview(root: &Path) -> CognitiveDaemonPreview {
    match archon_cognitive::CognitiveDaemon::status(root, 120_000) {
        Ok(status) => {
            let state = status.state;
            CognitiveDaemonPreview {
                running: status.running,
                stale: status.stale,
                stop_requested: status.stop_requested,
                ticks_run: state.as_ref().map(|state| state.ticks_run).unwrap_or(0),
                pid: state.as_ref().map(|state| state.pid),
                last_heartbeat_at: state.map(|state| state.last_heartbeat_at.to_rfc3339()),
            }
        }
        Err(_) => daemon_preview_for_missing_store(),
    }
}

fn daemon_preview_for_missing_store() -> CognitiveDaemonPreview {
    CognitiveDaemonPreview {
        running: false,
        stale: false,
        stop_requested: false,
        ticks_run: 0,
        pid: None,
        last_heartbeat_at: None,
    }
}

fn path_probe(label: &str, path: &Path) -> PathProbe {
    PathProbe {
        label: label.into(),
        path: path.to_string_lossy().to_string(),
        exists: path.exists(),
        files: if path.is_dir() { count_files(path) } else { 0 },
        bytes: path.metadata().map(|metadata| metadata.len()).unwrap_or(0),
    }
}

fn count_files(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .map(|entries| entries.flatten().count() as u64)
        .unwrap_or(0)
}
