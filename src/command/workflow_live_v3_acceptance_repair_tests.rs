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

struct Run {
    set: FrozenSet,
    store: WorkflowStore,
    runtime: WorkflowV2ScriptRuntime,
    universe: WorkflowV2TaskUniverse,
    run_id: String,
}

/// AC-F-001 passes as frozen; AC-F-002 was refuted by the judge at freeze.
fn run_fixture() -> Run {
    let set = frozen_set(&[
        ("AC-F-001", "test -f present", true),
        ("AC-F-002", "test -f missing", false),
    ]);
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
        tasks: vec![
            task("TASK-F-001", "AC-F-001"),
            task("TASK-F-002", "AC-F-002"),
        ],
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

async fn stage(
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
    assert!(!record.has_remediable_failures());
    assert!(record.final_round);
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
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
