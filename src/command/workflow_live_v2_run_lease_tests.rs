//! Issue 252: a generic V2 run has one executor. A resume takes the run's
//! executor lease; a live holder refuses it with its pid and leaves the run
//! directory unchanged, and a dead holder is recovered with a recorded event.

use super::*;
use crate::command::tui_workflow_ui_sink::default_workflow_ui_sink;
use crate::command::workflow_task_root_reclaim::begin_execution;
use archon_workflow::WorkflowAgentOutcome;

struct PanicLlm;

#[async_trait::async_trait]
impl WorkflowLlmClient for PanicLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("this workflow makes no agent call")
    }
}

/// Answers every agent call, and records whether the run's executor lease was
/// held while the call ran.
struct LeaseProbeLlm {
    store_root: PathBuf,
    held: std::sync::Mutex<Vec<bool>>,
}

impl LeaseProbeLlm {
    fn new(store: &WorkflowStore) -> Arc<Self> {
        Arc::new(Self {
            store_root: store.root().to_path_buf(),
            held: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn observed(&self) -> Vec<bool> {
        self.held.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for LeaseProbeLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        for entry in fs::read_dir(&self.store_root)
            .into_iter()
            .flatten()
            .flatten()
        {
            let id = entry.file_name().to_string_lossy().to_string();
            if entry.path().join("state.json").is_file() {
                let free = crate::command::workflow_executor_lease::acquire(&entry.path(), &id);
                self.held.lock().unwrap().push(free.is_err());
            }
        }
        Ok(WorkflowAgentOutcome {
            content: "probe answered".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: Some("end_turn".to_string()),
        })
    }
}

const SCRIPT: &str = "export default async function workflow(w) { await w.agent(\"probe\", { task: \"answer once\" }); return \"done\"; }";

/// A generic V2 run its executor left `Running`.
fn running_run(root: &Path) -> (WorkflowStore, String) {
    let store = WorkflowStore::project(root);
    let spec = archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "generic-executor-lease".to_string(),
        task: "one executor per generic run".to_string(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let run = store.create_run(spec).expect("seed run");
    WorkflowBundle::create_for_run(&store, &run, SCRIPT, WorkflowBundleOrigin::GeneratedHarness)
        .expect("bundle");
    store
        .write_run_json(
            &run.id,
            GENERATED_V2_METADATA_PATH,
            &GeneratedV2Metadata {
                schema_version: "workflow-generated-v2-metadata-v1".to_string(),
                run_kind: None,
                fixed_identity: None,
                observer_snapshot: None,
                generated_kind: None,
                scaffold_hash: Some(workflow_scaffold_hash(SCRIPT)),
                generated_scaffold: None,
                task_universe: None,
                script_args: None,
                governed_learning_context: Vec::new(),
                generated_config: None,
                tuning_decisions: Vec::new(),
                shape_decisions: Vec::new(),
                script_lifecycle: Some(false),
            },
        )
        .expect("metadata");
    let mut running = store.load_state(&run.id).expect("state");
    running.status = RunStatus::Running;
    store.save_state(&running).expect("running");
    (store, run.id)
}

async fn resume(
    root: &Path,
    store: &WorkflowStore,
    run_id: &str,
    llm: Arc<dyn WorkflowLlmClient>,
) -> Result<Option<String>> {
    let (ui_sink, rx) = default_workflow_ui_sink();
    std::mem::forget(rx);
    resume_generated_v2_workflow(
        root,
        store,
        run_id,
        llm,
        ui_sink,
        Vec::new(),
        LiveApprovalMode::CliYes,
        true,
        &archon_core::config::LearningConfig::default(),
    )
    .await
}

/// Every file of the run directory but the lease, with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in fs::read_dir(&next).expect("read dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if !path.ends_with(crate::command::workflow_executor_lease::LEASE) {
                out.insert(path.clone(), fs::read(&path).expect("read"));
            }
        }
    }
    out
}

#[tokio::test]
async fn a_second_resume_is_refused_while_the_executor_lives() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, run_id) = running_run(temp.path());
    let live = begin_execution(&store, &run_id).expect("the live executor's lease");
    live.record_executor().expect("executor record");
    let before = snapshot(&store.run_dir(&run_id));
    let error = resume(temp.path(), &store, &run_id, Arc::new(PanicLlm))
        .await
        .expect_err("a live executor refuses a second resume")
        .to_string();
    assert!(error.contains("is live"), "{error}");
    assert!(
        error.contains(&format!("process {}", std::process::id())),
        "{error}"
    );
    assert_eq!(snapshot(&store.run_dir(&run_id)), before);
}

#[tokio::test]
async fn a_resume_after_the_executor_died_records_the_recovery_and_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, run_id) = running_run(temp.path());
    // The executor took the lease and died: the kernel released it.
    let dead = begin_execution(&store, &run_id).expect("the dead executor's lease");
    dead.record_executor().expect("executor record");
    drop(dead);
    let probe = LeaseProbeLlm::new(&store);
    resume(temp.path(), &store, &run_id, probe.clone())
        .await
        .expect("a dead executor's run resumes")
        .expect("a generic V2 run");
    let observed = probe.observed();
    assert!(
        !observed.is_empty() && observed.iter().all(|held| *held),
        "the resume holds the lease: {observed:?}"
    );
    let events = fs::read_to_string(store.run_dir(&run_id).join("events.jsonl")).expect("events");
    assert!(events.contains("stale_owner_recovered"), "{events}");
    assert_ne!(
        store.load_state(&run_id).expect("state").status,
        RunStatus::Running
    );
}

#[tokio::test]
async fn a_launch_holds_the_lease_while_it_executes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::project(temp.path());
    let plan = WorkflowScriptPlan::generated(
        "probe the executor lease",
        SCRIPT,
        Vec::new(),
        None,
        GeneratedWorkflowConfig::default(),
        &archon_core::config::LearningConfig::default(),
    )
    .expect("plan");
    let probe = LeaseProbeLlm::new(&store);
    let (ui_sink, rx) = default_workflow_ui_sink();
    std::mem::forget(rx);
    run_generated_v2_workflow(
        temp.path(),
        &store,
        plan,
        "probe the executor lease".to_string(),
        probe.clone(),
        ui_sink,
        Vec::new(),
        LiveApprovalMode::CliYes,
        true,
        false,
        &archon_core::config::LearningConfig::default(),
    )
    .await
    .expect("launch");
    let observed = probe.observed();
    assert!(
        !observed.is_empty() && observed.iter().all(|held| *held),
        "the launch holds the lease: {observed:?}"
    );
}
