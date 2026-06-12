use std::collections::BTreeSet;

use chrono::Utc;
use serde_json::json;

use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::persistence;
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::store::WorkflowStore;

/// AC-WC-010 resume classification for coordinated implementation items.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumeClassification {
    /// Already Applied or IdempotentNoop — skip execution + apply.
    pub skip: Vec<String>,
    /// Failed or PendingApply — re-execute via the coordinator.
    pub reexecute: Vec<String>,
    /// Conflicted — surface to the operator; do NOT auto-retry.
    pub surfaced_conflicts: Vec<String>,
    /// No prior manifest — a new item, run normally.
    pub fresh: Vec<String>,
}

/// Classify each item for resume using the persisted manifest status
/// (TASK-WC-006 `resume_status`). Pure — drives the resume decision.
pub fn classify_resume(
    run_root: &std::path::Path,
    stage_id: &str,
    item_ids: &[String],
) -> ResumeClassification {
    use crate::write_coordinator::patch_apply::{ApplyResumeStatus, resume_status};

    let mut out = ResumeClassification::default();
    for item in item_ids {
        match resume_status(item, run_root, stage_id) {
            ApplyResumeStatus::Applied | ApplyResumeStatus::IdempotentNoop => {
                out.skip.push(item.clone());
            }
            ApplyResumeStatus::Failed(_) | ApplyResumeStatus::PendingApply => {
                out.reexecute.push(item.clone());
            }
            ApplyResumeStatus::Conflicted => out.surfaced_conflicts.push(item.clone()),
            ApplyResumeStatus::NotPersisted => out.fresh.push(item.clone()),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleAction {
    Resume,
    Pause,
    Cancel,
    RestartStage(String),
    RestartItem {
        stage_id: String,
        item_id: String,
    },
    ForceAcceptStage {
        stage_id: String,
        forced_by: String,
        rationale: String,
        source: String,
    },
}

#[derive(Debug, Clone)]
pub struct LifecycleController {
    store: WorkflowStore,
}

impl LifecycleController {
    pub fn new(store: WorkflowStore) -> Self {
        Self { store }
    }

    pub fn apply(&self, run_id: &str, action: LifecycleAction) -> WorkflowResult<WorkflowRun> {
        let mut run = self.store.load_state(run_id)?;
        let forced_record = forced_accept_record(&action);
        let event = apply_action(&mut run, action)?;
        run.updated_at = Utc::now();
        self.store.save_state(&run)?;
        if let Some(record) = forced_record {
            persistence::record_forced_accept(
                &self.store,
                &run.id,
                &record.stage_id,
                &record.forced_by,
                &record.rationale,
                &record.source,
            )?;
        }
        emit_lifecycle_event(&self.store, &run.id, event)?;
        Ok(run)
    }
}

struct ForcedAcceptRecord {
    stage_id: String,
    forced_by: String,
    rationale: String,
    source: String,
}

fn forced_accept_record(action: &LifecycleAction) -> Option<ForcedAcceptRecord> {
    match action {
        LifecycleAction::ForceAcceptStage {
            stage_id,
            forced_by,
            rationale,
            source,
        } => Some(ForcedAcceptRecord {
            stage_id: stage_id.clone(),
            forced_by: forced_by.clone(),
            rationale: rationale.clone(),
            source: source.clone(),
        }),
        _ => None,
    }
}

fn apply_action(
    run: &mut WorkflowRun,
    action: LifecycleAction,
) -> WorkflowResult<(WorkflowEventKind, serde_json::Value)> {
    match action {
        LifecycleAction::Resume => {
            run.status = RunStatus::Running;
            Ok((WorkflowEventKind::Resumed, json!({"action": "resume"})))
        }
        LifecycleAction::Pause => {
            run.status = RunStatus::Paused;
            Ok((WorkflowEventKind::Paused, json!({"action": "pause"})))
        }
        LifecycleAction::Cancel => {
            run.status = RunStatus::Cancelled;
            Ok((WorkflowEventKind::Cancelled, json!({"action": "cancel"})))
        }
        LifecycleAction::RestartStage(stage_id) => {
            rewind_stage(run, &stage_id)?;
            Ok((
                WorkflowEventKind::Resumed,
                json!({"action": "restart_stage", "stage": stage_id}),
            ))
        }
        LifecycleAction::RestartItem { stage_id, item_id } => {
            rewind_item(run, &stage_id, &item_id)?;
            Ok((
                WorkflowEventKind::Resumed,
                json!({"action": "restart_item", "stage": stage_id, "item": item_id}),
            ))
        }
        LifecycleAction::ForceAcceptStage {
            stage_id,
            forced_by,
            rationale,
            source,
        } => {
            force_accept_stage(run, &stage_id)?;
            Ok((
                WorkflowEventKind::ForcedAccepted,
                json!({
                    "action": "force_accept_stage",
                    "stage": stage_id,
                    "forced_by": forced_by,
                    "rationale": rationale,
                    "source": source,
                }),
            ))
        }
    }
}

fn rewind_item(run: &mut WorkflowRun, stage_id: &str, item_id: &str) -> WorkflowResult<()> {
    if !run.stages.contains_key(stage_id) {
        return Err(WorkflowError::SpecInvalid(format!(
            "unknown stage {stage_id}"
        )));
    }
    let item = run
        .items
        .get(item_id)
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("unknown item {item_id}")))?;
    if item.stage_id != stage_id {
        return Err(WorkflowError::SpecInvalid(format!(
            "item {item_id} does not belong to stage {stage_id}"
        )));
    }
    // Per-item rewind: drop only this item and re-open its owning stage plus
    // transitive dependents. Sibling accepted items are preserved so they are
    // served from cache on the next run (AC-US3-02, EC-DWF-18).
    run.items.remove(item_id);
    let rewind_ids = dependent_stage_ids(run, stage_id);
    for id in &rewind_ids {
        if let Some(state) = run.stages.get_mut(id) {
            state.status = StageStatus::Pending;
            state.error = None;
            state.started_at = None;
            state.completed_at = None;
            state.quality_score = None;
            // Dependent stages (not the item's own stage) are fully reset.
            if id != stage_id {
                state.artifacts.clear();
            }
        }
    }
    // Drop items belonging to dependent stages (but not siblings in this stage).
    run.items.retain(|_, item| {
        item.stage_id == stage_id || !rewind_ids.contains(item.stage_id.as_str())
    });
    run.status = RunStatus::Running;
    Ok(())
}

fn rewind_stage(run: &mut WorkflowRun, stage_id: &str) -> WorkflowResult<()> {
    if !run.stages.contains_key(stage_id) {
        return Err(WorkflowError::SpecInvalid(format!(
            "unknown stage {stage_id}"
        )));
    }
    let rewind_ids = dependent_stage_ids(run, stage_id);
    for id in &rewind_ids {
        if let Some(state) = run.stages.get_mut(id) {
            state.status = StageStatus::Pending;
            state.error = None;
            state.started_at = None;
            state.completed_at = None;
            state.quality_score = None;
            state.artifacts.clear();
        }
    }
    run.items
        .retain(|_, item| !rewind_ids.contains(item.stage_id.as_str()));
    run.status = RunStatus::Running;
    Ok(())
}

fn dependent_stage_ids(run: &WorkflowRun, stage_id: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::from([stage_id.to_string()]);
    loop {
        let before = ids.len();
        for stage in &run.spec.stages {
            if ids.contains(&stage.id) {
                continue;
            }
            if stage.depends_on.iter().any(|dep| ids.contains(dep)) {
                ids.insert(stage.id.clone());
            }
        }
        if ids.len() == before {
            return ids;
        }
    }
}

fn force_accept_stage(run: &mut WorkflowRun, stage_id: &str) -> WorkflowResult<()> {
    let state = run
        .stages
        .get_mut(stage_id)
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("unknown stage {stage_id}")))?;
    state.status = StageStatus::ForcedAccepted;
    state.error = None;
    state.completed_at = Some(Utc::now());
    if run.stages.values().all(|stage| stage.is_terminal()) {
        run.status = RunStatus::Completed;
    } else {
        run.status = RunStatus::Running;
    }
    Ok(())
}

fn emit_lifecycle_event(
    store: &WorkflowStore,
    run_id: &str,
    event: (WorkflowEventKind, serde_json::Value),
) -> WorkflowResult<()> {
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(run_id, seq, event.0, event.1)?;
    Ok(())
}
