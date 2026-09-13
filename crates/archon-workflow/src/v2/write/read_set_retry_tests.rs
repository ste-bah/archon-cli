use super::*;
use crate::WorkflowV2HostMethod;
use std::sync::atomic::{AtomicUsize, Ordering};

struct ReadThenDisconnect(AtomicUsize);
#[async_trait::async_trait]
impl WorkflowAgentDispatch for ReadThenDisconnect {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn run_call(
        &self,
        _task: &str,
        _: Option<String>,
        execution: &WorkflowV2CallExecution,
        _: &WorkflowV2AgentAdapter,
        store: Option<&WorkflowV2ResultStore>,
        _: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        let store = store.unwrap();
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            let path = crate::v2::write_read_set::path(store, &execution.call.id);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                path,
                "{\"path\":\"src/needed.rs\",\"offset\":7,\"limit\":12}\n",
            )
            .unwrap();
            return Err(WorkflowError::port("response_failed: connection reset"));
        }
        assert!(store.load_branch_outcomes().unwrap().is_empty());
        // Real request builder prefers options.task over the fallback argument.
        let prompt = execution.call.options.task.as_deref().unwrap();
        assert!(
            prompt.contains("src/needed.rs")
                && prompt.contains("offset=7")
                && prompt.contains("limit=12"),
            "{prompt}"
        );
        Ok(WorkflowV2Result::accepted("continued with orientation"))
    }
}

#[tokio::test]
async fn write_read_set_immediate_transport_retry_uses_current_sidecar_before_wave_save() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let dispatch = ReadThenDisconnect(AtomicUsize::new(0));
    let branch = WorktreeBranchExecution {
        id: "agents-2-0".into(),
        role: "coder".into(),
        input_hash: None,
        workspace_root: temp.path().to_path_buf(),
        execution: WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "agents-2-0".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: Some(WorkflowV2WriteMode::Worktree),
                options: Default::default(),
            },
            input: serde_json::json!({}),
            depends_on: Vec::new(),
        },
        refresh: None,
    };
    run_worktree_branch_agent(
        "implement",
        None,
        &dispatch,
        &store,
        WorkflowV2AgentAdapter::new(),
        &branch,
        None,
    )
    .await
    .unwrap();
    assert_eq!(dispatch.0.load(Ordering::SeqCst), 2);
}
