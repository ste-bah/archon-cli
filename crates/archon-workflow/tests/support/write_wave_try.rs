//! A write wave whose outcome is returned rather than unwrapped, so a test
//! can see a wave that ends the run's way -- a pause -- instead of a result.
use super::*;

impl Fixture {
    /// One wave of `items`, as [`Fixture::wave`] runs it, its outcome
    /// returned: a wave a branch paused is the pause.
    pub async fn try_wave(
        &self,
        id: &str,
        items: Vec<(Vec<&str>, Edits)>,
    ) -> WorkflowResult<WorkflowV2Result> {
        let (call, branches) = self.wave_branches(id, items);
        let task_ids = self.item_task_ids.clone();
        let judged = (None, &[][..], &[][..]);
        self.try_wave_for(&self.v2, call, branches, judged, task_ids, false)
            .await
            .0
    }

    /// The fan-out call `id` over `items`: each is (declared targets, edits)
    /// and becomes branch `<id>-<index>`.
    pub(super) fn wave_branches(
        &self,
        id: &str,
        items: Vec<(Vec<&str>, Edits)>,
    ) -> (WorkflowV2HostCall, Vec<(WorkflowV2FanoutItem, Edits)>) {
        let call = WorkflowV2HostCall {
            id: id.into(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".into()),
                task: Some("Implement the item now.".into()),
                target_files_from_item: true,
                ..Default::default()
            },
        };
        let mut branches = Vec::new();
        for (index, (targets, edits)) in items.into_iter().enumerate() {
            let branch_id = format!("{id}-{index}");
            let mut branch = call.clone();
            branch.id = branch_id.clone();
            branch.method = WorkflowV2HostMethod::Implementation;
            branch.options.target_files = targets.iter().map(|t| (*t).to_string()).collect();
            let item = WorkflowV2FanoutItem::read_only(
                branch_id.clone(),
                "coder",
                branch,
                json!({"item": {"item_id": branch_id, "canonical_task_ids": self.item_task_ids,
                    "target_files": targets, "work_type": "implementation"}}),
            );
            branches.push((item, edits));
        }
        (call, branches)
    }

    /// [`Fixture::wave_for`], its outcome returned.
    pub(super) async fn try_wave_for(
        &self,
        store: &WorkflowV2ResultStore,
        call: WorkflowV2HostCall,
        branches: Vec<(WorkflowV2FanoutItem, Edits)>,
        (audit, failing, rejecting): (Option<AuditScript>, &[&str], &[&str]),
        task_ids: Vec<String>,
        panic_on_work: bool,
    ) -> (WorkflowResult<WorkflowV2Result>, Vec<String>) {
        let per_branch = branches
            .iter()
            .map(|(item, edits)| (item.id.clone(), edits.clone()))
            .collect();
        let branches = branches.into_iter().map(|(item, _)| item).collect();
        let dispatch = Scripted {
            per_branch,
            prompts: Mutex::new(vec![]),
            stamps: self.stamps.clone(),
            audit: audit.map(|script| (self.audit_runtime(), script)),
            failing: failing.iter().map(|id| (*id).to_string()).collect(),
            task_ids,
            panic_on_work,
            rejecting: rejecting.iter().map(|id| (*id).to_string()).collect(),
            shell: self.shell.clone(),
        };
        let result = run_write_capable_v2_fanout(
            "fallback objective",
            Some(self.repo.to_str().unwrap()),
            WorkflowV2CallExecution {
                call,
                input: json!({}),
                depends_on: vec![],
            },
            WorkflowV2AgentAdapter::new(),
            &dispatch,
            store,
            &self.store,
            &self.run,
            true,
            branches,
            self.universe.as_ref(),
            None,
        )
        .await;
        (result, dispatch.prompts.into_inner().unwrap())
    }
}
