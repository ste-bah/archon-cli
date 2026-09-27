//! Audited writes share capture/apply enforcement regardless of scheduling mode.
use super::*;

pub(super) async fn run(
    ctx: WriteFanoutContext<'_>,
    branches: Vec<WorkflowV2FanoutItem>,
    mut plan: WorkflowV2WritePlan,
    reused: Vec<WorkflowV2Result>,
) -> WorkflowResult<WorkflowV2Result> {
    // Preserve the planner's serial/coordinated wave boundaries. Only the
    // workspace/apply mechanism changes: no audited writer mutates canonical
    // source before its actual patch has passed the audit gate.
    assign_worktrees(&mut plan, ctx.v2_store, &ctx.execution.call.id);
    run_worktree_v2_write_fanout(ctx, branches, plan, reused).await
}

fn assign_worktrees(plan: &mut WorkflowV2WritePlan, store: &WorkflowV2ResultStore, call_id: &str) {
    for wave in &mut plan.waves {
        for assignment in &mut wave.assignments {
            if assignment.worktree_path.is_some() {
                continue;
            }
            assignment.worktree_path = Some(
                store
                    .root()
                    .join("worktrees")
                    .join(workspace_segment(call_id))
                    .join(workspace_segment(&assignment.item_id))
                    .display()
                    .to_string(),
            );
        }
    }
}

fn workspace_segment(id: &str) -> String {
    if cfg!(windows) {
        blake3::hash(id.as_bytes()).to_hex()[..16].to_string()
    } else {
        sanitize_v2_path_segment(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audited_writes_keep_the_planners_workspace_assignment() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let id = "review-remediate-cross-task-001-task-00-8a1b2c3d-1";
        let planner = WorkflowV2WritePlanner::new(temp.path().join("compact"));
        let mut plan = planner
            .plan(&[WorkflowV2WriteItem::new(
                id,
                WorkflowV2WriteMode::Worktree,
                vec!["owned.txt".into()],
            )])
            .unwrap();
        let before = plan.clone();
        assign_worktrees(&mut plan, &store, id);
        assert_eq!(
            plan, before,
            "audit must not discard the planner's bounded path"
        );
    }
    #[test]
    fn audited_serial_and_coordinated_waves_receive_distinct_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        for mode in [
            WorkflowV2WriteMode::Serial,
            WorkflowV2WriteMode::Coordinated,
        ] {
            let planner = WorkflowV2WritePlanner::new(temp.path().join("unused"));
            let items: Vec<_> = ["a", "b"]
                .into_iter()
                .map(|id| WorkflowV2WriteItem::new(id, mode, vec![format!("{id}.txt")]))
                .collect();
            let mut plan = planner.plan(&items).unwrap();
            let before = plan.clone();
            assign_worktrees(&mut plan, &store, "long-call");
            let paths: Vec<_> = plan
                .waves
                .iter()
                .flat_map(|w| &w.assignments)
                .map(|a| a.worktree_path.as_ref().unwrap())
                .collect();
            assert_eq!(paths.len(), 2);
            assert_ne!(paths[0], paths[1]);
            for path in paths {
                assert!(Path::new(path).starts_with(store.root().join("worktrees")));
            }
            for wave in &mut plan.waves {
                for assignment in &mut wave.assignments {
                    assignment.worktree_path = None;
                }
            }
            assert_eq!(plan, before, "audit changes no scheduling or ownership");
        }
    }
}
