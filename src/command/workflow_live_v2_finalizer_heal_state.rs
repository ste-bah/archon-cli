//! What the pre-commit re-entry loop judges and keeps between attempts.
//!
//! - `situation`: what an observation judged, with volatile text masked, so a
//!   retry's minted ids, timestamps and evidence or scratch paths are never
//!   taken for progress.
//! - `ReopenLedger`: the re-entries made so far for the uncommitted outcome,
//!   persisted under the run so a pause (or a crash) during re-entry resumes
//!   the count instead of starting it again. The terminal commit clears it.

use std::sync::LazyLock;

use archon_workflow::{
    FinalizationRecordV1, RunEndAcceptanceObserverSnapshotV1, WorkflowError, WorkflowResult,
    WorkflowStore,
};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::super::super::workflow_live_v2_script::WorkflowV2ScriptSummary;
use super::super::require_generation_owner;

/// Where the uncommitted outcome's re-entries are kept.
pub(super) const REOPEN_LEDGER_PATH: &str = "v2/run-end-reopens.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ReopenLedger {
    /// The observation failure each re-entry answered, oldest first.
    pub(super) reopens: Vec<String>,
    /// Every situation an observation failed in.
    pub(super) situations: Vec<String>,
    /// The fewest failing checks a completed observation has reported.
    #[serde(default)]
    pub(super) fewest: Option<usize>,
    /// Re-entries since the last observation that reported fewer failing
    /// checks than every one before it: what the runaway guard counts.
    #[serde(default)]
    pub(super) since_progress: usize,
}

impl ReopenLedger {
    pub(super) fn load(store: &WorkflowStore, run_id: &str) -> WorkflowResult<Self> {
        let path = store.run_dir(run_id).join(REOPEN_LEDGER_PATH);
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = std::fs::read(&path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub(super) fn save(
        &self,
        store: &WorkflowStore,
        run_id: &str,
        expected_generation: Option<u64>,
    ) -> WorkflowResult<()> {
        store.with_run_lock(run_id, |locked| {
            require_generation_owner(locked, run_id, expected_generation)?;
            locked.write_run_json(run_id, REOPEN_LEDGER_PATH, self)
        })
    }
}

/// The terminal commit decided the outcome the ledger was kept for.
pub(in super::super) fn clear_reopen_ledger(
    store: &WorkflowStore,
    run_id: &str,
) -> WorkflowResult<()> {
    let path = store.run_dir(run_id).join(REOPEN_LEDGER_PATH);
    match std::fs::remove_file(&path) {
        Err(source) if source.kind() != std::io::ErrorKind::NotFound => {
            Err(WorkflowError::Io { path, source })
        }
        _ => Ok(()),
    }
}

/// What the observation judged: its failure (volatile text masked), the
/// pin's bytes, and the outcome the acceptance gate recorded.
pub(super) fn situation(
    store: &WorkflowStore,
    snapshot: &RunEndAcceptanceObserverSnapshotV1,
    reason: &str,
    summary: &WorkflowV2ScriptSummary,
    record: &FinalizationRecordV1,
) -> String {
    let pin = super::super::super::workflow_run_end_snapshot::project_root(store)
        .map(|project| {
            crate::command::workflow_task_set::acceptance_pin_path(
                project,
                std::path::Path::new(&snapshot.canonical_task_root_identity),
            )
        })
        .and_then(|path| std::fs::read(path).ok())
        .map(|bytes| archon_workflow::task_set_contract::content_digest(&bytes))
        .unwrap_or_else(|| "unreadable".to_string());
    let gate = record.acceptance_gate.as_ref().map(|gate| {
        (
            gate.contract_present,
            gate.failing_check_ids.clone(),
            gate.operational_errors
                .iter()
                .map(|error| masked(error))
                .collect::<Vec<_>>(),
        )
    });
    format!("{}|{pin}|{:?}|{gate:?}", masked(reason), summary.status)
}

static EVIDENCE_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(?:[A-Za-z]:)?[/\\][^\s'"`,;()\[\]{}]*(?:evidence|scratch)[^\s'"`,;()\[\]{}]*"#,
    )
    .expect("evidence path pattern")
});
static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
        .expect("uuid pattern")
});
static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2})?")
        .expect("timestamp pattern")
});

/// `text` with evidence and scratch paths, hyphenated uuids, timestamps,
/// digit runs and hex runs of eight or more characters replaced.
pub(super) fn masked(text: &str) -> String {
    let text = EVIDENCE_PATH.replace_all(text, "<path>");
    let text = UUID.replace_all(&text, "<id>");
    let text = TIMESTAMP.replace_all(&text, "<time>");
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        let volatile = token.chars().all(|c| c.is_ascii_digit())
            || (token.len() >= 8 && token.chars().all(|c| c.is_ascii_hexdigit()));
        out.push_str(if volatile { "#" } else { token });
        token.clear();
    };
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            if !token.is_empty() {
                flush(&mut token, &mut out);
            }
            out.push(c);
        }
    }
    if !token.is_empty() {
        flush(&mut token, &mut out);
    }
    out
}

/// The checks the observation recorded failing (its `policy_shadow` rows).
pub(super) fn shadowed_checks(store: &WorkflowStore, run_id: &str) -> Vec<String> {
    let path = store
        .run_dir(run_id)
        .join(super::super::super::workflow_run_end_observer::RUN_END_OBSERVER_RECORDS_PATH);
    let mut ids = std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|row| row["record_kind"] == "policy_shadow")
        .filter_map(|row| row["acceptance_id"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

/// A failed observation's native evidence is kept beside the next one's,
/// never overwritten by it.
pub(super) fn keep_native_evidence(
    store: &WorkflowStore,
    run_id: &str,
    prior: usize,
) -> WorkflowResult<()> {
    let native = store
        .run_dir(run_id)
        .join("observer/native-observation.json");
    if !native.exists() {
        return Ok(());
    }
    let kept = native.with_file_name(format!(
        "native-observation.pre-commit-{}-{}.json",
        prior + 1,
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::rename(&native, &kept).map_err(|source| WorkflowError::Io {
        path: native.clone(),
        source,
    })
}
