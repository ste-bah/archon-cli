//! Per-run deterministic test hooks and a realistic fixed-driver fixture.
use super::*;
use std::sync::Mutex;

type Hook = Box<dyn FnOnce() + Send>;
static STOP: Mutex<BTreeMap<PathBuf, Hook>> = Mutex::new(BTreeMap::new());
static UNWIND: Mutex<BTreeMap<PathBuf, Hook>> = Mutex::new(BTreeMap::new());

pub(in super::super) fn on_unwind(path: PathBuf, hook: Hook) {
    UNWIND.lock().unwrap().insert(path, hook);
}

pub(in super::super) fn unwind(path: PathBuf) {
    let hook = UNWIND.lock().unwrap().remove(&path);
    if let Some(hook) = hook {
        hook();
    }
}

pub(crate) fn seed_fixed(store: &WorkflowStore, run_id: &str, root: &Path) {
    let task_root = root.join("tasks");
    fs::create_dir_all(&task_root).unwrap();
    let task_root = task_root
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let project_root = task_root.parent().unwrap();
    store
        .write_run_json(
            run_id,
            crate::command::workflow_decompose_state::FIXED_STATE_PATH,
            &archon_workflow::FixedDecompositionStateV1 {
                schema_version: archon_workflow::FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION,
                run_kind: archon_workflow::WorkflowRunKind::FixedDecompositionV1,
                identity: archon_workflow::FixedRunIdentityV1 {
                    template_version: "fixed-decomposition-v1".into(),
                    starting_binary_revision: "test".into(),
                    script_digest: "script".into(),
                    catalog_digest: "catalog".into(),
                    project_root_identity: project_root.display().to_string(),
                    prd_identity: project_root.join("PRD.md").display().to_string(),
                    task_root_identity: task_root.display().to_string(),
                },
                phase: archon_workflow::DecompositionPhase::Identity,
                attempts: Default::default(),
                dispositions: Default::default(),
                log_path: task_root.join(".decompose.log").display().to_string(),
            },
        )
        .unwrap();
}

pub(crate) fn save_fixed_metadata(store: &WorkflowStore, run_id: &str, plan: &WorkflowScriptPlan) {
    let state: archon_workflow::FixedDecompositionStateV1 = serde_json::from_slice(
        &fs::read(
            store
                .run_dir(run_id)
                .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH),
        )
        .unwrap(),
    )
    .unwrap();
    save_fixed_decomposition_metadata(store, run_id, plan, &state.identity).unwrap();
}

pub(super) struct PendingReply;
#[async_trait::async_trait]
impl WorkflowLlmClient for PendingReply {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        std::future::pending().await
    }
}

pub(in super::super) fn on_stop(path: PathBuf, hook: Hook) {
    STOP.lock().unwrap().insert(path, hook);
}
pub(in super::super) fn stop(path: PathBuf) {
    let hook = STOP.lock().unwrap().remove(&path);
    if let Some(hook) = hook {
        hook();
    }
}
