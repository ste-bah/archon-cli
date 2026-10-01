//! A round that meets a refuted frozen check repairs the contract, not tasks.

use super::*;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use crate::command::workflow_task_set::republish::test_fixture::{
    FrozenSet, assert_only_named_entries_changed, frozen_set,
};
use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;
use archon_workflow::{WorkflowSpec, WorkflowV2HostCall, WorkflowV2HostMethod};

pub(super) struct Run {
    pub(super) set: FrozenSet,
    pub(super) store: WorkflowStore,
    runtime: WorkflowV2ScriptRuntime,
    universe: WorkflowV2TaskUniverse,
    pub(super) run_id: String,
}

/// AC-F-001 passes as frozen; AC-F-002 was refuted by the judge at freeze.
fn run_fixture() -> Run {
    run_fixture_with(&[
        ("AC-F-001", "test -f present", true),
        ("AC-F-002", "test -f missing", false),
    ])
}

/// A run over `checks` (id, command, accepted), TASK-F-00n implementing the
/// n-th, with a `present` file in the project.
pub(super) fn run_fixture_with(checks: &[(&str, &str, bool)]) -> Run {
    let set = frozen_set(checks);
    std::fs::write(set.project.path().join("present"), "x").unwrap();
    let task = |id: &str, implements: &str| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: set.tasks.join(format!("{id}.md")).display().to_string(),
        implements: vec![implements.into()],
        ..WorkflowV2TaskUniverseTask::default()
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![set.tasks.display().to_string()],
        tasks: checks
            .iter()
            .map(|(id, _, _)| task(&id.replace("AC-", "TASK-"), id))
            .collect(),
    };
    let store = WorkflowStore::project(set.project.path());
    let root = set.project.path().display().to_string();
    let run = store
        .create_run(WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "acceptance".into(),
            task: "acceptance".into(),
            target_repository_root: Some(root.clone()),
            max_parallelism: 1,
            max_agents: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    Run {
        run_id: run.id,
        runtime: WorkflowV2ScriptRuntime {
            target_repository_root: Some(root),
            generated_config: Default::default(),
        },
        set,
        store,
        universe,
    }
}

fn round_one() -> WorkflowV2CallExecution {
    let (options, _) = archon_workflow::v2::script::parse_script_options(&serde_json::json!({
        "tool": ACCEPTANCE_STAGE_TOOL, "round": 1, "maxRounds": 3, "checkIds": [],
    }))
    .unwrap();
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "acceptance-contract-run-1".into(),
            method: WorkflowV2HostMethod::Tool,
            write_mode: None,
            options,
        },
        input: serde_json::json!({}),
        depends_on: Vec::new(),
    }
}

pub(super) async fn stage(
    run: &Run,
    client: &ScriptedAuthorJudge,
) -> (WorkflowV2Result, AcceptanceRoundRecordV1) {
    let result = run_acceptance_stage(
        &run.runtime,
        &round_one(),
        &run.store,
        &run.run_id,
        Some(&run.universe),
        Some(client),
    )
    .await
    .expect("round runs");
    let (record, _) =
        archon_workflow::v2::acceptance_stage::latest_round_record(&run.store.run_dir(&run.run_id))
            .unwrap()
            .unwrap();
    (result, record)
}

#[tokio::test]
async fn a_refuted_check_is_reauthored_rejudged_and_passes_in_the_same_round() {
    let run = run_fixture();
    let before = run.set.contract_bytes();
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f present && test -s present"),
        |_, _| true,
    );
    let (result, record) = stage(&run, &client).await;
    assert_eq!(record.contract_repairs.len(), 1);
    assert!(
        record.contract_repairs[0].repaired,
        "{:?}",
        record.contract_repairs
    );
    assert_eq!(
        record.contract_repairs[0].check_ids,
        vec!["AC-F-002".to_string()]
    );
    assert!(record.failing_checks().is_empty(), "{:?}", record.checks);
    assert_eq!(record.passed_check_ids(), vec!["AC-F-001", "AC-F-002"]);
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(result.data["final"], true);
    let named = ["AC-F-002".to_string()].into_iter().collect();
    assert_only_named_entries_changed(&before, &run.set.contract_bytes(), &named);
    archon_workflow::task_skeleton::validate_full_chain(&run.set.tasks, &run.set.pin())
        .expect("the republished chain verifies");
    assert_eq!(
        *client.resolved_agents.lock().unwrap(),
        vec![archon_core::agents::harness::ACCEPTANCE_REAUTHOR_AGENT.to_string()],
        "the in-round repair launches a key the agent registry resolves in every project"
    );
}

#[tokio::test]
async fn an_unrepairable_refuted_check_is_a_blocking_contract_defect_owned_by_no_task() {
    let run = run_fixture();
    let before = run.set.chain_bytes();
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, &format!("test -f present{attempt}")),
        |_, _| false,
    );
    let (result, record) = stage(&run, &client).await;
    assert_eq!(client.authored(), 3, "the in-round repair is bounded");
    assert_eq!(run.set.chain_bytes(), before, "nothing republished");
    assert!(!record.contract_repairs[0].repaired);
    let defect = record
        .checks
        .iter()
        .find(|check| check.check_id == "AC-F-002")
        .unwrap();
    assert!(defect.contract_defect);
    assert!(defect.failing());
    assert!(
        defect.owning_tasks.is_empty(),
        "a contract defect is never routed to the implementing task"
    );
    let text = defect.operational_error.as_deref().unwrap();
    assert!(
        text.contains(&format!(
            "archon workflow freeze-acceptance --reauthor AC-F-002 --tasks {}",
            run.set.tasks.canonicalize().unwrap().display()
        )),
        "{text}"
    );
    assert_eq!(record.passed_check_ids(), vec!["AC-F-001"]);
    // Batch O: never a task's, but the host's to re-author again next
    // round, so the loop is not over on the first failed repair.
    assert!(record.task_remediable_check_ids().is_empty());
    assert!(record.has_remediable_failures());
    assert!(!record.final_round);
    assert!(record.blocks_completion());
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(
        result.data["contract_defect_check_ids"],
        serde_json::json!(["AC-F-002"])
    );
}

#[test]
fn the_launcher_refuses_a_run_bound_to_a_refuted_check() {
    let run = run_fixture();
    let error = super::super::workflow_run_end_snapshot::refuse_unaccepted_launch(
        &run.store,
        Some(&run.universe),
    )
    .expect_err("refuted check bound at launch")
    .to_string();
    assert!(error.contains("--reauthor AC-F-002"), "{error}");
}

#[tokio::test]
async fn after_an_in_round_repair_the_scratch_guardian_verifies_the_republished_chain() {
    use crate::command::acceptance_scratch_guardian::validate_selected;
    use archon_workflow::acceptance_scratch::ScratchPolicy;
    let run = run_fixture();
    let context = super::exec::resolve_context(
        &run.store,
        &run.run_id,
        run.runtime.target_repository_root.as_deref(),
        Some(&run.universe),
    )
    .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let binding = crate::command::acceptance_scratch_policy::NativeBinding {
        policy: ScratchPolicy {
            repository: context.repository.clone(),
            project: context.project.clone(),
            task_root: context.task_root.clone(),
            scratch_parent: scratch.path().to_path_buf(),
            project_inputs: Vec::new(),
            project_input_excludes: Vec::new(),
            combined: false,
            toolchain_path: String::new(),
            environment: Default::default(),
            environment_allowlist: Vec::new(),
            cargo_seed: None,
            timeout_secs: 60,
            output_bytes: 4096,
            scratch_bytes: 1 << 20,
            build_cache: None,
        },
        source_commit: "0".repeat(40),
    };
    let request = |label: &str| {
        super::exec::scratch_request(
            &context,
            &binding,
            "0".repeat(40),
            scratch.path().join(label),
        )
        .unwrap()
    };
    let before_repair = request("stale");
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f present && test -s present"),
        |_, _| true,
    );
    let (_, record) = stage(&run, &client).await;
    assert!(record.contract_repairs[0].repaired);
    let selection = Some(["AC-F-002".to_string()].into_iter().collect());
    let stale = validate_selected(&before_repair, &selection)
        .expect_err("a pin captured before the repair is refused")
        .to_string();
    assert!(stale.contains("pin changed"), "{stale}");
    // The request the stage builds at observation time reads the pin anew.
    let (_, _, refs) = validate_selected(&request("fresh"), &selection)
        .expect("the guardian verifies the republished chain");
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].command_digest,
        archon_workflow::task_set_contract::content_digest(b"test -f present && test -s present")
    );
}

#[tokio::test]
async fn the_real_launch_path_refuses_a_refuted_contract_before_creating_a_run() {
    let run = run_fixture();
    let runs = || std::fs::read_dir(run.store.root()).unwrap().count();
    let before = runs();
    let plan = super::super::WorkflowScriptPlan::generated(
        "launch over a refuted contract",
        "export default async function workflow(w) { await w.checkpoint(\"noop\", {}); }",
        Vec::new(),
        Some(run.universe.clone()),
        archon_core::config::GeneratedWorkflowConfig::default(),
        &archon_core::config::LearningConfig::default(),
    )
    .expect("plan resolves");
    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::bounded_workflow_ui_sink(16);
    let client: std::sync::Arc<dyn archon_workflow::WorkflowLlmClient> = std::sync::Arc::new(
        ScriptedAuthorJudge::new(|_, _| unreachable!("no agent runs"), |_, _| true),
    );
    let error = super::super::run_generated_v2_workflow(
        run.set.project.path(),
        &run.store,
        plan,
        "launch over a refuted contract".into(),
        client,
        ui_sink,
        Vec::new(),
        super::super::LiveApprovalMode::CliYes,
        true,
        true,
        &archon_core::config::LearningConfig::default(),
    )
    .await
    .expect_err("the launcher refuses")
    .to_string();
    assert!(error.contains("refusing to launch"), "{error}");
    assert!(error.contains("--reauthor AC-F-002"), "{error}");
    assert_eq!(runs(), before, "no run was created");
}
