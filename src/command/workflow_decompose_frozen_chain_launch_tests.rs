use super::*;
use archon_workflow::task_set_contract::{ACCEPTANCE_LOCK_FILE, content_digest};

const PRD: &str = "# PRD X\n\n## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Prove the fixture. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is proven. |\n";

struct Fixture {
    _temp: tempfile::TempDir,
    project: PathBuf,
    tasks: PathBuf,
    store: WorkflowStore,
}

/// A project whose acceptance contract is frozen, with `command(project)`
/// as its one check.
fn frozen(command: Option<fn(&Path) -> String>) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project = temp
        .path()
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let prd = project.join("prds/PRD-X.md");
    let tasks = project.join("tasks/PRD-X");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(&prd, PRD).unwrap();
    // A (commitless) git checkout, so the project can serve as the
    // repository a launch is grounded in.
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    super::super::test_support::freeze_acceptance(&project, &prd, &tasks);
    if let Some(command) = command {
        // Re-freeze the same contract with another check: contract, lock
        // and pin agree, so the frozen chain verifies.
        let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
        let mut contract: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        contract["acceptance"][0]["check"]["command"] = command(&project).into();
        let bytes = serde_json::to_vec_pretty(&contract).unwrap();
        let digest = content_digest(&bytes);
        std::fs::write(&path, &bytes).unwrap();
        for file in [
            tasks.join(ACCEPTANCE_LOCK_FILE),
            crate::command::workflow_task_set::acceptance_pin_path(&project, &tasks),
        ] {
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            let key = if value.get("digest").is_some() {
                "digest"
            } else {
                "acceptance_digest"
            };
            value[key] = digest.clone().into();
            std::fs::write(&file, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        }
    }
    super::super::frozen_chain_snapshot(&project, &prd, &tasks).expect("the fixture verifies");
    let store = WorkflowStore::project(&project);
    Fixture {
        _temp: temp,
        project,
        tasks,
        store,
    }
}

/// An earlier run of this project that froze the live contract with a
/// freeze-acceptance judged by logic `version`.
fn recorded_freeze(fixture: &Fixture, version: u32) {
    let run = fixture
        .store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "earlier".into(),
            task: "earlier decomposition".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    fixture
        .store
        .write_run_json(
            &run.id,
            "decomposition/state.json",
            &serde_json::json!({"identity": {
                "task_root_identity": fixture.tasks.to_string_lossy().replace('\\', "/")
            }}),
        )
        .unwrap();
    let path = fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let bytes = std::fs::read(&path).unwrap();
    let record: archon_workflow::WorkflowV2CallRecord = serde_json::from_value(serde_json::json!({
        "run_id": run.id,
        "call": {"id": "freeze-acceptance-1", "method": "hostCommand",
                 "options": {"host_command": {"commandId": "freeze-acceptance", "stdin": "{}"}}},
        "attempt": 1, "started_at": "2026-10-10T10:00:00Z", "input_hash": "fixture",
        "status": "accepted",
        "result": {"status": "accepted", "summary": "fixture", "data": {
            "exitCode": 0, "stdout": "", "stderr": "", "stdoutBytes": 0, "stderrBytes": 0,
            "timedOut": false, "interrupted": false, "stdoutTruncated": false, "stderrTruncated": false,
            "gateEnvelope": {"schema_version": 1, "report": "fixture", "policy_findings": []},
            "publicationReceipt": {"schema_version": 1, "call_id": "freeze-acceptance-1",
                "command_id": "freeze-acceptance", "committed_at": "2026-10-10T10:00:00Z",
                "entries": [{"relative_path": ACCEPTANCE_CONTRACT_FILE, "destination_path": path,
                    "byte_len": bytes.len(), "blake3": content_digest(&bytes)}]},
            "postcondition": {"satisfied": true, "summary": "fixture"},
            "logicVersion": version
        }}
    }))
    .unwrap();
    archon_workflow::WorkflowV2ResultStore::new(fixture.store.run_dir(&run.id).join("v2"))
        .save_call_record(&record)
        .unwrap();
}

fn decide(fixture: &Fixture) -> Result<Vec<String>> {
    let catalog =
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("fixture")
            .unwrap();
    launch_acceptance_decision(
        true,
        &fixture.store,
        (&fixture.project, &fixture.tasks, &fixture.project),
        &catalog,
    )
}

fn current() -> u32 {
    crate::command::workflow_host_command_logic::versions()[FREEZE_ACCEPTANCE]
}

#[test]
fn a_current_recorded_freeze_is_verified_in_place_and_says_so() {
    let fixture = frozen(None);
    recorded_freeze(&fixture, current());
    let lines = decide(&fixture).unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("frozen_chain_launch acceptance=verify reason=run "));
    assert!(
        lines[0].contains("replays under this build's logic"),
        "{}",
        lines[0]
    );
}

#[test]
fn a_stale_recorded_freeze_whose_checks_pass_current_rules_is_verified_and_named() {
    let fixture = frozen(None);
    recorded_freeze(&fixture, current() - 1);
    let lines = decide(&fixture).unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("reason=stale recorded freeze logic"),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].contains(&format!(
            "logicVersion recorded={} current={}",
            current() - 1,
            current()
        )),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].contains("pass all 1 frozen check(s)"),
        "{}",
        lines[0]
    );
}

/// A check that names the project root by its absolute path: the freeze
/// refuses it since Issue 366, an older judge accepted it.
fn names_live_root(project: &Path) -> String {
    format!("test -f {}/marker", project.display())
}

#[test]
fn a_stale_recorded_freeze_whose_check_a_current_rule_refuses_stops_the_launch() {
    let fixture = frozen(Some(names_live_root));
    recorded_freeze(&fixture, current() - 1);
    let error = decide(&fixture).unwrap_err().to_string();
    assert!(error.contains("refuse check(s) AC-X-001"), "{error}");
    assert!(error.contains("(stale: run "), "{error}");
    assert!(error.contains("--reauthor"), "{error}");
}

#[test]
fn a_current_recorded_freeze_is_not_re_validated() {
    // Its judge ran this build's rules already; the refusal above is only
    // for logic this build cannot vouch for.
    let fixture = frozen(Some(names_live_root));
    recorded_freeze(&fixture, current());
    assert!(decide(&fixture).is_ok());
}

#[test]
fn an_unrecorded_freeze_is_verified_under_current_rules() {
    let fixture = frozen(None);
    let lines = decide(&fixture).unwrap();
    assert!(
        lines[0].contains("reason=unrecorded recorded freeze logic"),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].contains("no run of this project recorded"),
        "{}",
        lines[0]
    );
    let refused = frozen(Some(names_live_root));
    let error = decide(&refused).unwrap_err().to_string();
    assert!(error.contains("(unrecorded: no run"), "{error}");
}

#[test]
fn nothing_is_decided_for_a_stage_that_is_not_frozen() {
    let fixture = frozen(None);
    let catalog =
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("fixture")
            .unwrap();
    let lines = launch_acceptance_decision(
        false,
        &fixture.store,
        (&fixture.project, &fixture.tasks, &fixture.project),
        &catalog,
    )
    .unwrap();
    assert!(lines.is_empty());
}

/// Stops a launch at provider construction: everything before it ran.
struct LaunchBarrier;

#[async_trait::async_trait(?Send)]
impl archon_workflow::WorkflowLlmClientFactory for LaunchBarrier {
    async fn build_client(
        &self,
        _request: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>>
    {
        Err(archon_workflow::WorkflowError::port("barrier observed"))
    }
}

async fn launch(fixture: &Fixture) -> anyhow::Error {
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.repository_root = Some(fixture.project.clone());
    crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &fixture.project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &config,
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        &LaunchBarrier,
    )
    .await
    .unwrap_err()
}

#[tokio::test]
async fn a_launch_on_a_frozen_check_this_builds_rules_refuse_stops_before_any_run() {
    let fixture = frozen(Some(names_live_root));
    let error = format!("{:#}", launch(&fixture).await);
    assert!(error.contains("refuse check(s) AC-X-001"), "{error}");
    assert!(!error.contains("barrier observed"), "{error}");
    assert!(
        fixture.store.list_runs().unwrap().is_empty(),
        "a refused launch creates no run"
    );
}

#[tokio::test]
async fn a_launch_on_a_frozen_contract_logs_its_decision() {
    let fixture = frozen(None);
    let error = format!("{:#}", launch(&fixture).await);
    assert!(error.contains("barrier observed"), "{error}");
    let log = std::fs::read_to_string(fixture.tasks.join(".decompose.log")).unwrap();
    assert!(
        log.contains(
            "frozen_chain_launch acceptance=verify reason=unrecorded recorded freeze logic"
        ),
        "{log}"
    );
    let run = fixture.store.list_runs().unwrap().pop().unwrap();
    let inventory = fixture
        .store
        .run_dir(&run.id)
        .join(crate::command::workflow_decompose_requirement_inventory::INVENTORY_PATH);
    let inventory: serde_json::Value =
        serde_json::from_slice(&std::fs::read(inventory).unwrap()).unwrap();
    assert_eq!(
        inventory["ids"],
        serde_json::json!(["REQ-X-001"]),
        "{inventory}"
    );
    assert_eq!(inventory["recorded_at"], "launch");
}
