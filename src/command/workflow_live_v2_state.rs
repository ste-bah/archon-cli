use anyhow::Result;
use archon_workflow::run::StageState;
use archon_workflow::{
    RunStatus, StageStatus, WorkflowStore, WorkflowV2HostCall, WorkflowV2ResultStore,
    WorkflowV2Status,
};

pub(super) fn mark_v2_call_running(
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
) -> archon_workflow::WorkflowResult<()> {
    let mut run = store.load_state(run_id)?;
    if matches!(run.status, RunStatus::Paused | RunStatus::Cancelled) {
        return Ok(());
    }
    run.status = RunStatus::Running;
    let stage = run
        .stages
        .entry(call_id.to_string())
        .or_insert_with(|| StageState::pending(call_id));
    stage.status = StageStatus::Running;
    stage.error = None;
    stage.started_at.get_or_insert_with(chrono::Utc::now);
    stage.completed_at = None;
    run.mark_updated();
    store.save_state_preserving_control(&run)
}

/// Persist a terminal run status to state.json on the paths that never reach
/// `sync_v2_summary_to_run`.
///
/// A paused, cancelled or errored run returns early from the generated-run
/// executor, so its final status was only ever written to events.jsonl while
/// state.json kept its last value — `Running`. Observed live: a run whose event
/// log recorded `terminal_status: failed` still reported `status: running` with
/// 78 stages "running" hours after the process died. Everything that reads run
/// status is then wrong: resume logic, status queries, the resumable guard, and
/// any monitoring. Stages are left untouched (there is no summary on these
/// paths); only the run-level status is reconciled.
pub(super) fn persist_terminal_run_status(
    store: &WorkflowStore,
    run_id: &str,
    status: RunStatus,
) -> archon_workflow::WorkflowResult<()> {
    let mut run = store.load_state(run_id)?;
    if run.status == status {
        return Ok(());
    }
    run.status = status;
    run.mark_updated();
    store.save_state_preserving_control(&run)
}

pub(super) fn sync_v2_summary_to_run(
    store: &WorkflowStore,
    run_id: &str,
    calls: &[WorkflowV2HostCall],
    v2_store: &WorkflowV2ResultStore,
    status: WorkflowV2Status,
) -> Result<()> {
    let mut run = store.load_state(run_id)?;
    for call in calls {
        let call_status = v2_store
            .load_call_record(&call.id)?
            .map(|record| record.status)
            .unwrap_or(WorkflowV2Status::Pending);
        let stage = run
            .stages
            .entry(call.id.clone())
            .or_insert_with(|| StageState::pending(&call.id));
        stage.status = stage_status_from_v2(call_status);
        if matches!(
            stage.status,
            StageStatus::Accepted
                | StageStatus::Blocked
                | StageStatus::NeedsReview
                | StageStatus::Failed
                | StageStatus::Cancelled
        ) {
            stage.completed_at = Some(chrono::Utc::now());
        }
    }
    let completed_cleanly = matches!(status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop);
    let now = chrono::Utc::now();
    for spec_stage in &run.spec.stages {
        if !is_dynamic_template_stage(spec_stage)
            || calls.iter().any(|call| call.id == spec_stage.id)
        {
            continue;
        }
        let template_matched_runtime_call =
            dynamic_stage_prefix(spec_stage).is_some_and(|prefix| {
                calls
                    .iter()
                    .any(|call| call.id.starts_with(prefix.as_str()))
            });
        if !completed_cleanly && !template_matched_runtime_call {
            continue;
        }
        let stage = run
            .stages
            .entry(spec_stage.id.clone())
            .or_insert_with(|| StageState::pending(&spec_stage.id));
        if matches!(stage.status, StageStatus::Pending | StageStatus::Running) {
            stage.status = StageStatus::Skipped;
            stage.completed_at = Some(now);
            stage.error = Some(
                "dynamic host-call template; runtime-discovered call instances carry the concrete result"
                    .to_string(),
            );
        }
    }
    run.status = run_status_from_v2(status);
    run.mark_updated();
    store.save_state(&run)?;
    Ok(())
}

fn is_dynamic_template_stage(stage: &archon_workflow::StageSpec) -> bool {
    stage
        .extra
        .get("dynamic_template")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn dynamic_stage_prefix(stage: &archon_workflow::StageSpec) -> Option<String> {
    stage
        .extra
        .get("dynamic_id_prefix")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn stage_status_from_v2(status: WorkflowV2Status) -> StageStatus {
    match status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => StageStatus::Accepted,
        WorkflowV2Status::Blocked => StageStatus::Blocked,
        WorkflowV2Status::NeedsReview => StageStatus::NeedsReview,
        WorkflowV2Status::Failed => StageStatus::Failed,
        WorkflowV2Status::Cancelled => StageStatus::Cancelled,
        WorkflowV2Status::Pending => StageStatus::Pending,
        WorkflowV2Status::Running => StageStatus::Running,
    }
}

fn run_status_from_v2(status: WorkflowV2Status) -> RunStatus {
    match status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => RunStatus::Completed,
        WorkflowV2Status::Blocked => RunStatus::Blocked,
        WorkflowV2Status::NeedsReview => RunStatus::NeedsReview,
        WorkflowV2Status::Failed => RunStatus::Failed,
        WorkflowV2Status::Cancelled => RunStatus::Cancelled,
        WorkflowV2Status::Pending | WorkflowV2Status::Running => RunStatus::Running,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use archon_workflow::{
        RetryPolicy, StageKind, StageSpec, WorkflowSpec, WorkflowV2CallRecord, WorkflowV2HostCall,
        WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
    };

    use super::*;

    #[test]
    fn mark_running_inserts_runtime_discovered_v2_stage() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(temp.path().join("workflows"));
        let run = store.create_run(spec_with_stage("static-plan")).unwrap();

        mark_v2_call_running(&store, &run.id, "implementation-wave-1").unwrap();

        let run = store.load_state(&run.id).unwrap();
        let stage = run.stages.get("implementation-wave-1").unwrap();
        assert_eq!(stage.status, StageStatus::Running);
        assert!(stage.started_at.is_some());
    }

    #[test]
    fn sync_summary_inserts_runtime_discovered_v2_stage() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(temp.path().join("workflows"));
        let run = store
            .create_run(spec_with_stage("implementation-wave-dynamic"))
            .unwrap();
        let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
        let call = WorkflowV2HostCall {
            id: "implementation-wave-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        };
        let record = WorkflowV2CallRecord::new(
            v2_store.run_id(),
            call.clone(),
            1,
            "input-hash".to_string(),
            WorkflowV2Result::accepted("accepted dynamic call"),
            Vec::new(),
        );
        v2_store.save_call_record(&record).unwrap();

        sync_v2_summary_to_run(
            &store,
            &run.id,
            &[call],
            &v2_store,
            WorkflowV2Status::Accepted,
        )
        .unwrap();

        let run = store.load_state(&run.id).unwrap();
        let stage = run.stages.get("implementation-wave-1").unwrap();
        assert_eq!(stage.status, StageStatus::Accepted);
        assert!(stage.completed_at.is_some());
    }

    #[test]
    fn sync_summary_skips_dynamic_template_when_runtime_instance_exists() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(temp.path().join("workflows"));
        let run = store
            .create_run(spec_with_dynamic_template_stage(
                "implementation-wave-dynamic-12345678",
                "implementation-wave-",
            ))
            .unwrap();
        let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
        let call = WorkflowV2HostCall {
            id: "implementation-wave-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        };
        let record = WorkflowV2CallRecord::new(
            v2_store.run_id(),
            call.clone(),
            1,
            "input-hash".to_string(),
            WorkflowV2Result::accepted("accepted dynamic call"),
            Vec::new(),
        );
        v2_store.save_call_record(&record).unwrap();

        sync_v2_summary_to_run(
            &store,
            &run.id,
            &[call],
            &v2_store,
            WorkflowV2Status::Accepted,
        )
        .unwrap();

        let run = store.load_state(&run.id).unwrap();
        assert_eq!(
            run.stages
                .get("implementation-wave-dynamic-12345678")
                .unwrap()
                .status,
            StageStatus::Skipped
        );
        assert_eq!(
            run.stages.get("implementation-wave-1").unwrap().status,
            StageStatus::Accepted
        );
    }

    #[test]
    fn sync_summary_skips_dynamic_template_when_clean_loop_does_not_execute() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(temp.path().join("workflows"));
        let run = store
            .create_run(spec_with_dynamic_template_stage(
                "remediation-dynamic-12345678",
                "remediation-",
            ))
            .unwrap();
        let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));

        sync_v2_summary_to_run(&store, &run.id, &[], &v2_store, WorkflowV2Status::Accepted)
            .unwrap();

        let run = store.load_state(&run.id).unwrap();
        assert_eq!(
            run.stages
                .get("remediation-dynamic-12345678")
                .unwrap()
                .status,
            StageStatus::Skipped
        );
    }

    fn spec_with_stage(stage_id: &str) -> WorkflowSpec {
        spec_with_stage_extra(stage_id, BTreeMap::new())
    }

    fn spec_with_dynamic_template_stage(stage_id: &str, prefix: &str) -> WorkflowSpec {
        let mut extra = BTreeMap::new();
        extra.insert(
            "dynamic_template".to_string(),
            serde_json::Value::Bool(true),
        );
        extra.insert(
            "dynamic_id_prefix".to_string(),
            serde_json::Value::String(prefix.to_string()),
        );
        spec_with_stage_extra(stage_id, extra)
    }

    fn spec_with_stage_extra(
        stage_id: &str,
        extra: BTreeMap<String, serde_json::Value>,
    ) -> WorkflowSpec {
        WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "test".to_string(),
            task: "test".to_string(),
            target_repository_root: Some("/repo".to_string()),
            max_parallelism: 8,
            max_agents: 32,
            stages: vec![StageSpec {
                id: stage_id.to_string(),
                kind: StageKind::Agent,
                task: Some("test".to_string()),
                agent: None,
                foreach: None,
                reducer: None,
                tool: None,
                depends_on: Vec::new(),
                provider_tier: None,
                retry: RetryPolicy::default(),
                input: serde_json::Value::Null,
                model: None,
                provider: None,
                expected_target_files: Vec::new(),
                verify_command: None,
                max_parallelism: None,
                item_kind: None,
                filter: None,
                extra,
            }],
            permissions: BTreeMap::new(),
            learning_hooks: Vec::new(),
        }
    }
}
