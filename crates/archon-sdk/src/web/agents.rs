//! `GET /api/agents/live` — what the agents in THIS process are doing.
//!
//! Two registries hold live agents and neither is serialisable:
//! `BACKGROUND_AGENTS` owns a `JoinHandle` + `CancellationToken` per agent, and
//! `TASK_MANAGER` owns cancellation tokens for `TaskCreate`-spawned work. A
//! separate `archon web` process therefore cannot see either of them at any
//! price — this endpoint only returns anything when the server runs inside the
//! session (see `WebServer::attached`).
//!
//! This is a snapshot of current state, not an append-only log, so it is
//! polled by the client rather than streamed: streaming it would mean either
//! diffing or resending the world on every change.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use archon_tools::background_agents::{AgentStatus, BACKGROUND_AGENTS};
use archon_tools::task_manager::{TASK_MANAGER, TaskStatus};

use super::{AppState, check_auth, live::now_ms};

/// One live agent, projected narrowly. Handles are deliberately absent: they
/// are not serialisable and the dashboard has no use for them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebAgentActivity {
    pub id: String,
    /// Which registry this came from: `background` or `task`.
    pub kind: String,
    /// Human label — the task description, or the agent id for background
    /// agents, whose registry carries no description.
    pub label: String,
    pub status: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebAgentActivitySnapshot {
    pub agents: Vec<WebAgentActivity>,
    pub observed_at_ms: u128,
    /// `false` when the server was started standalone, where both registries
    /// are empty by construction rather than because nothing is running.
    pub attached: bool,
}

/// First time this server observed a background agent.
///
/// `BackgroundAgentRegistryApi` exposes `iter_running`/`get` but not
/// `spawned_at` — the field lives on the handle, which the trait does not hand
/// out. Rather than reach around the registry, the projection reports elapsed
/// since first observation, which is the quantity this layer can actually
/// measure. `TASK_MANAGER` entries carry `created_at`, so those report true
/// elapsed and do not go through here.
#[derive(Clone, Default)]
pub struct WebAgentObserver {
    first_seen: Arc<Mutex<HashMap<String, Instant>>>,
}

impl WebAgentObserver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Elapsed since first observation, recording `now` on first sight.
    /// `live` is the full set still present, so entries for agents that have
    /// gone terminal are dropped instead of accumulating for the process
    /// lifetime.
    fn elapsed_ms(&self, id: &str, live: &[String]) -> u64 {
        let mut seen = self.first_seen.lock().expect("agent observer poisoned");
        seen.retain(|key, _| live.iter().any(|alive| alive == key));
        let started = seen.entry(id.to_string()).or_insert_with(Instant::now);
        started.elapsed().as_millis() as u64
    }
}

pub(crate) async fn live_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let snapshot = WebAgentActivitySnapshot {
        agents: collect_agents(&state.agents),
        observed_at_ms: now_ms(),
        attached: state.attached,
    };
    (StatusCode::OK, Json(snapshot)).into_response()
}

fn collect_agents(observer: &WebAgentObserver) -> Vec<WebAgentActivity> {
    let running: Vec<String> = BACKGROUND_AGENTS
        .iter_running()
        .into_iter()
        .map(|id| id.to_string())
        .collect();

    let mut agents: Vec<WebAgentActivity> = BACKGROUND_AGENTS
        .iter_running()
        .into_iter()
        .filter_map(|id| {
            // Re-read the status: an agent can go terminal between
            // `iter_running` and here, and a dashboard that keeps showing a
            // dead agent as live is worse than one that shows it late.
            let status = BACKGROUND_AGENTS.get(&id)?;
            if status.is_terminal() {
                return None;
            }
            let id = id.to_string();
            let elapsed_ms = observer.elapsed_ms(&id, &running);
            Some(WebAgentActivity {
                label: id.clone(),
                id,
                kind: "background".to_string(),
                status: background_status_label(status).to_string(),
                elapsed_ms,
            })
        })
        .collect();

    // TaskCreate-spawned agents live in a different registry and never reach
    // BACKGROUND_AGENTS, so both have to be read to answer "what is running".
    agents.extend(TASK_MANAGER.list_tasks().into_iter().filter_map(|task| {
        if is_terminal_task(&task.status) {
            return None;
        }
        Some(WebAgentActivity {
            id: task.id,
            kind: "task".to_string(),
            label: task.description,
            status: task.status.to_string().to_lowercase(),
            elapsed_ms: elapsed_since_ms(task.created_at.timestamp_millis()),
        })
    }));

    agents.sort_by(|left, right| right.elapsed_ms.cmp(&left.elapsed_ms));
    agents
}

fn background_status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Running => "running",
        AgentStatus::Finished => "finished",
        AgentStatus::Failed => "failed",
        AgentStatus::Cancelled => "cancelled",
    }
}

fn is_terminal_task(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Stopped
    )
}

/// Taken as epoch millis so this module does not need a `chrono` dependency
/// just to name the type on `TaskInfo::created_at`.
fn elapsed_since_ms(created_at_ms: i64) -> u64 {
    (now_ms() as i64).saturating_sub(created_at_ms).max(0) as u64
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WebAgentActivity::decl(&cfg)),
        exported(WebAgentActivitySnapshot::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}
