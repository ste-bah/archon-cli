//! Observe-only run-end acceptance evaluation.
//!
//! This host reads a launch-opted frozen chain after terminal persistence. It
//! evaluates only the shared pure commandless-floor kernel. Command checks,
//! nested typed verifier commands, and residual fail-closed text are recorded
//! as operational deferrals and are never rendered or executed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceCheck, AcceptanceContract, AcceptancePin,
    RESIDUAL_GAPS_FILE, ResidualGapRecord, validate_residual_gaps,
};
use archon_workflow::{
    DeclarativeFloorEvaluation, ObserverAuthority, RUN_END_OBSERVER_EXPECTED_ARTIFACT_PATHS,
    RUN_END_OBSERVER_SNAPSHOT_SCHEMA_VERSION, RunEndObserverOutcomeV1, WorkflowError,
    WorkflowEventKind, WorkflowEventLog, WorkflowResult, WorkflowStore,
    collect_declarative_floor_facts, evaluate_declarative_floor,
};
use serde::Serialize;

use super::workflow_live_v2_finalizer::{RunEndObserverContext, WorkflowRunEndObserver};

pub(super) const RUN_END_OBSERVER_RECORDS_PATH: &str = "observer/run-end-acceptance.jsonl";

pub(super) struct FixedRunEndAcceptanceObserver {
    store: WorkflowStore,
}

impl FixedRunEndAcceptanceObserver {
    pub(super) fn new(store: WorkflowStore) -> Self {
        Self { store }
    }
}

#[derive(Serialize)]
struct ObserverRecord<'a> {
    schema_version: &'static str,
    record_kind: &'static str,
    acceptance_id: &'a str,
    authority: &'static str,
    detail: &'a str,
}

struct OwnedObserverRecord {
    record_kind: &'static str,
    acceptance_id: String,
    detail: String,
}

impl OwnedObserverRecord {
    fn shadow(acceptance_id: &str, detail: &str) -> Self {
        Self {
            record_kind: "policy_shadow",
            acceptance_id: acceptance_id.to_string(),
            detail: detail.to_string(),
        }
    }

    fn deferral(acceptance_id: &str, detail: &str) -> Self {
        Self {
            record_kind: "operational_deferral",
            acceptance_id: acceptance_id.to_string(),
            detail: detail.to_string(),
        }
    }

    fn as_borrowed(&self) -> ObserverRecord<'_> {
        ObserverRecord {
            schema_version: "run-end-acceptance-observer-v1",
            record_kind: self.record_kind,
            acceptance_id: &self.acceptance_id,
            authority: "observe_only",
            detail: &self.detail,
        }
    }
}

impl WorkflowRunEndObserver for FixedRunEndAcceptanceObserver {
    fn observe(
        &self,
        context: &RunEndObserverContext<'_>,
    ) -> WorkflowResult<RunEndObserverOutcomeV1> {
        let task_root = validate_expected_root(context)?;
        let project_root = project_root(&self.store)?;
        let pin_path =
            crate::command::workflow_task_set::acceptance_pin_path(project_root, &task_root);
        let pin: AcceptancePin = read_json(&pin_path)?;
        validate_snapshot_identity(context, &pin)?;
        archon_workflow::task_skeleton::validate_full_chain(&task_root, &pin)
            .map_err(|error| WorkflowError::StateCorrupt(error.to_string()))?;
        let contract: AcceptanceContract = read_json(&task_root.join(ACCEPTANCE_CONTRACT_FILE))?;
        let expected = contract
            .acceptance
            .iter()
            .map(|criterion| criterion.id.clone())
            .collect();
        archon_workflow::task_set_contract::validate_acceptance_bundle(
            &task_root,
            Some(&pin),
            &expected,
        )
        .map_err(|error| WorkflowError::StateCorrupt(error.to_string()))?;

        if !matches!(
            context.terminal_status,
            archon_workflow::WorkflowV2Status::Accepted
                | archon_workflow::WorkflowV2Status::Noop
                | archon_workflow::WorkflowV2Status::NeedsReview
        ) {
            return Err(WorkflowError::StateCorrupt(
                "run-end observer received an ineligible terminal status".to_string(),
            ));
        }
        let mut evaluated = 0usize;
        let mut findings = 0usize;
        let mut deferrals = 0usize;
        let mut pending_records = Vec::new();
        let mut pending_shadow_events = BTreeSet::new();
        let mut passed_floor_ids = BTreeSet::new();
        for criterion in contract.acceptance.iter().chain(&contract.supplementary) {
            match &criterion.check {
                AcceptanceCheck::Command { .. } => {
                    deferrals += 1;
                    pending_records.push(OwnedObserverRecord::deferral(
                        &criterion.id,
                        "command-bearing acceptance check deferred in R2a",
                    ));
                }
                AcceptanceCheck::Floor { contract }
                    if archon_workflow::declarative_floor_deferral_reason(contract).is_some() =>
                {
                    deferrals += 1;
                    pending_records.push(OwnedObserverRecord::deferral(
                        &criterion.id,
                        "command-bearing or advanced floor deferred in R2a",
                    ));
                }
                AcceptanceCheck::Floor { contract } => {
                    evaluated += 1;
                    let facts = collect_declarative_floor_facts(project_root, contract)?;
                    match evaluate_declarative_floor(contract, &facts) {
                        DeclarativeFloorEvaluation::Passed => {
                            passed_floor_ids.insert(criterion.id.clone());
                        }
                        DeclarativeFloorEvaluation::Failed {
                            findings: floor_findings,
                        } => {
                            for detail in &floor_findings {
                                findings += 1;
                                pending_records
                                    .push(OwnedObserverRecord::shadow(&criterion.id, detail));
                                pending_shadow_events.insert(criterion.id.clone());
                            }
                        }
                        DeclarativeFloorEvaluation::Deferred { .. } => {
                            return Err(WorkflowError::StateCorrupt(
                                "declarative-floor eligibility changed during observation"
                                    .to_string(),
                            ));
                        }
                    }
                }
            }
        }
        let residual_records =
            validate_optional_residuals(&task_root, &contract, &passed_floor_ids)?;
        deferrals += residual_records.len();
        pending_records.extend(residual_records);
        write_records(&self.store, context.run_id, &pending_records)?;
        for acceptance_id in pending_shadow_events {
            emit_shadow_event(&self.store, context.run_id, &acceptance_id)?;
        }
        Ok(RunEndObserverOutcomeV1 {
            authority: ObserverAuthority::ObserveOnly,
            evaluated_floor_count: evaluated,
            policy_finding_count: findings,
            operational_deferral_count: deferrals,
        })
    }
}

fn validate_expected_root(context: &RunEndObserverContext<'_>) -> WorkflowResult<PathBuf> {
    let expected_paths = RUN_END_OBSERVER_EXPECTED_ARTIFACT_PATHS
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    if context.snapshot.schema_version != RUN_END_OBSERVER_SNAPSHOT_SCHEMA_VERSION
        || context.snapshot.expected_artifact_paths != expected_paths
    {
        return Err(WorkflowError::StateCorrupt(
            "run-end observer snapshot schema or expected artifact set is invalid".to_string(),
        ));
    }
    let declared = PathBuf::from(&context.snapshot.canonical_task_root_identity);
    let canonical = declared
        .canonicalize()
        .map_err(|source| WorkflowError::Io {
            path: declared.clone(),
            source,
        })?;
    if canonical.display().to_string() != context.snapshot.canonical_task_root_identity {
        return Err(WorkflowError::StateCorrupt(
            "launch-time task-root identity no longer resolves to the same canonical path"
                .to_string(),
        ));
    }
    Ok(canonical)
}

fn validate_snapshot_identity(
    context: &RunEndObserverContext<'_>,
    pin: &AcceptancePin,
) -> WorkflowResult<()> {
    let Some(expected) = context.snapshot.portable_acceptance_identity.as_ref() else {
        return Ok(());
    };
    if pin.freeze_event_id != expected.freeze_event_id
        || pin.acceptance_digest != expected.acceptance_digest
        || pin.skeleton_digest != expected.skeleton_digest
    {
        return Err(WorkflowError::StateCorrupt(
            "frozen acceptance identity differs from the launch-time observer snapshot".to_string(),
        ));
    }
    Ok(())
}

fn validate_optional_residuals(
    task_root: &Path,
    contract: &AcceptanceContract,
    passed_floor_ids: &BTreeSet<String>,
) -> WorkflowResult<Vec<OwnedObserverRecord>> {
    let path = task_root.join(RESIDUAL_GAPS_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let gaps: Vec<ResidualGapRecord> = read_json(&path)?;
    validate_residual_gaps(&contract.gap_policy, &gaps)
        .map_err(|error| WorkflowError::StateCorrupt(error.to_string()))?;
    for gap in &gaps {
        if passed_floor_ids.contains(&gap.acceptance_id) {
            return Err(WorkflowError::StateCorrupt(format!(
                "residual gap '{}' is stale: acceptance '{}' passed its commandless floor",
                gap.id, gap.acceptance_id
            )));
        }
    }
    Ok(gaps
        .iter()
        .map(|gap| {
            OwnedObserverRecord::deferral(
                &gap.acceptance_id,
                "residual fail-closed check deferred in R2a",
            )
        })
        .collect())
}

fn project_root(store: &WorkflowStore) -> WorkflowResult<&Path> {
    store
        .root()
        .parent()
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some(".archon"))
        .and_then(Path::parent)
        .ok_or_else(|| WorkflowError::StateCorrupt("workflow store has no project root".into()))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> WorkflowResult<T> {
    let bytes = std::fs::read(path).map_err(|source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(Into::into)
}

fn write_records(
    store: &WorkflowStore,
    run_id: &str,
    records: &[OwnedObserverRecord],
) -> WorkflowResult<()> {
    let mut bytes = Vec::new();
    for record in records {
        serde_json::to_writer(&mut bytes, &record.as_borrowed())?;
        bytes.push(b'\n');
    }
    store.write_run_file(run_id, RUN_END_OBSERVER_RECORDS_PATH, &bytes)
}

fn emit_shadow_event(
    store: &WorkflowStore,
    run_id: &str,
    acceptance_id: &str,
) -> WorkflowResult<()> {
    if observer_event_exists(
        store,
        run_id,
        "run_end_acceptance_shadow_observed",
        Some(acceptance_id),
    )? {
        return Ok(());
    }
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone())
        .emit(
            run_id,
            seq,
            WorkflowEventKind::RunEndAcceptanceShadowObserved,
            serde_json::json!({
                "event": "run_end_acceptance_shadow_observed",
                "acceptance_id": acceptance_id,
                "authority": "observe_only",
            }),
        )
        .map(|_| ())
}

fn observer_event_exists(
    store: &WorkflowStore,
    run_id: &str,
    event_label: &str,
    acceptance_id: Option<&str>,
) -> WorkflowResult<bool> {
    let path = store.events_path(run_id);
    let raw = std::fs::read_to_string(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let event: archon_workflow::WorkflowEvent = serde_json::from_str(line)?;
        if event
            .detail
            .get("event")
            .and_then(serde_json::Value::as_str)
            != Some(event_label)
        {
            continue;
        }
        if acceptance_id.is_none()
            || event
                .detail
                .get("acceptance_id")
                .and_then(serde_json::Value::as_str)
                == acceptance_id
        {
            return Ok(true);
        }
    }
    Ok(false)
}
