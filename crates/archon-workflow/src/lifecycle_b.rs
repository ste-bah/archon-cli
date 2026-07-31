fn item_evidence_paths(root: &Path, item: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["agent-outputs", "prompts"] {
        out.extend(nested_item_files(root, dir, item));
    }
    out.push(
        PathBuf::from("write-coordination")
            .join("items")
            .join(crate::store::safe_path_component(item)),
    );
    out
}

fn stage_artifact_paths(root: &Path, safe_stage: &str) -> Vec<PathBuf> {
    let dir = root.join("artifacts").join(safe_stage);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| {
            PathBuf::from("artifacts")
                .join(safe_stage)
                .join(entry.file_name())
        })
        .collect()
}

fn nested_item_files(root: &Path, dir: &str, item: &str) -> Vec<PathBuf> {
    let Ok(stages) = fs::read_dir(root.join(dir)) else {
        return Vec::new();
    };
    stages
        .flatten()
        .map(|stage| {
            PathBuf::from(dir)
                .join(stage.file_name())
                .join(format!("{item}.json"))
        })
        .filter(|rel| root.join(rel).exists())
        .collect()
}

fn move_run_path(root: &Path, src_rel: &Path, dst_rel: &Path) -> WorkflowResult<()> {
    let src = root.join(src_rel);
    let dst = root.join(dst_rel);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkflowError::io(parent, err))?;
    }
    fs::rename(&src, &dst).map_err(|err| WorkflowError::io(&dst, err))
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use chrono::Utc;

    use super::*;
    use crate::run::{ItemState, StageStatus};
    use crate::spec::{ArtifactPolicy, StageKind, StageSpec, WorkflowSpec};

    #[test]
    fn resume_reopens_cancelled_work_without_rewinding_accepted_work() {
        let mut run = WorkflowRun::new(test_spec(), PathBuf::from("/tmp/run"));
        run.status = RunStatus::Cancelled;
        run.stages.get_mut("accepted").unwrap().status = StageStatus::Accepted;
        let cancelled_stage = run.stages.get_mut("cancelled").unwrap();
        cancelled_stage.status = StageStatus::Cancelled;
        cancelled_stage.error = Some("cancelled".to_string());
        cancelled_stage.completed_at = Some(Utc::now());
        run.items.insert(
            "accepted-item".to_string(),
            ItemState {
                id: "accepted-item".to_string(),
                stage_id: "accepted".to_string(),
                status: StageStatus::Accepted,
                artifact: None,
                error: None,
            },
        );
        run.items.insert(
            "cancelled-item".to_string(),
            ItemState {
                id: "cancelled-item".to_string(),
                stage_id: "cancelled".to_string(),
                status: StageStatus::Cancelled,
                artifact: None,
                error: Some("cancelled".to_string()),
            },
        );

        apply_action(&mut run, LifecycleAction::Resume).unwrap();

        assert_eq!(run.status, RunStatus::Running);
        assert_eq!(
            run.stages.get("accepted").unwrap().status,
            StageStatus::Accepted
        );
        let resumed_stage = run.stages.get("cancelled").unwrap();
        assert_eq!(resumed_stage.status, StageStatus::Pending);
        assert_eq!(resumed_stage.error, None);
        assert_eq!(resumed_stage.completed_at, None);
        assert_eq!(
            run.items.get("accepted-item").unwrap().status,
            StageStatus::Accepted
        );
        let resumed_item = run.items.get("cancelled-item").unwrap();
        assert_eq!(resumed_item.status, StageStatus::Pending);
        assert_eq!(resumed_item.error, None);
    }

    fn test_spec() -> WorkflowSpec {
        WorkflowSpec {
            schema: crate::spec::WORKFLOW_SCHEMA.to_string(),
            name: "cancelled-resume".to_string(),
            task: "test cancelled resume".to_string(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            provider_tiers: BTreeMap::new(),
            stages: vec![test_stage("accepted"), test_stage("cancelled")],
            artifact_policy: ArtifactPolicy::default(),
            permissions: BTreeMap::new(),
            quality_gates: BTreeMap::new(),
            learning_hooks: Vec::new(),
        }
    }

    fn test_stage(id: &str) -> StageSpec {
        StageSpec {
            id: id.to_string(),
            kind: StageKind::Agent,
            task: Some(format!("run {id}")),
            agent: None,
            foreach: None,
            reducer: None,
            tool: None,
            condition: None,
            depends_on: Vec::new(),
            provider_tier: None,
            retry: Default::default(),
            input: serde_json::Value::Null,
            model: None,
            provider: None,
            expected_target_files: Vec::new(),
            verify_command: None,
            max_parallelism: None,
            item_kind: None,
            filter: None,
            extra: BTreeMap::new(),
        }
    }
}
