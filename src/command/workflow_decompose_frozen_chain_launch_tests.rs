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
    frozen_with_prd(command, PRD)
}

fn frozen_with_prd(command: Option<fn(&Path) -> String>, prd_contents: &str) -> Fixture {
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
    std::fs::write(&prd, prd_contents).unwrap();
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
        // Re-freeze the same contract with another check, judged by the
        // scripted judge: contract (in the host's own serialization), lock
        // and pin agree, so the frozen chain verifies and a per-check
        // repair can be judged like it.
        let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
        let mut contract: AcceptanceContract =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let entry = &mut contract.acceptance[0];
        entry.check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
            command: command(&project),
            cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
        };
        entry.judgment.sampling = Some(serde_json::json!({
            "temperature": 0.0,
            "model": "fixture-judge",
            "provider": crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER,
        }));
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

fn decide(fixture: &Fixture) -> Result<LaunchAcceptance> {
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
    let decision = decide(&fixture).unwrap();
    let lines = &decision.lines;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(decision.reauthor.is_empty());
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
    let decision = decide(&fixture).unwrap();
    let lines = &decision.lines;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(decision.reauthor.is_empty());
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
fn a_stale_recorded_freeze_routes_a_check_a_current_rule_refuses_to_re_authoring() {
    let fixture = frozen(Some(names_live_root));
    recorded_freeze(&fixture, current() - 1);
    let decision = decide(&fixture).expect("a refused frozen check never stops the launch");
    assert_eq!(
        decision.reauthor.keys().collect::<Vec<_>>(),
        vec!["AC-X-001"],
        "{decision:?}"
    );
    assert!(
        decision.reauthor["AC-X-001"].contains("acceptance entry 'AC-X-001' was refused"),
        "{decision:?}"
    );
    assert!(
        decision
            .recorded
            .starts_with("stale recorded freeze logic (run ")
    );
    assert!(
        decision.lines[0].starts_with(
            "frozen_chain_launch acceptance=reauthor ids=AC-X-001 kept=0 reason=stale"
        ),
        "{}",
        decision.lines[0]
    );
}

#[test]
fn a_current_recorded_freeze_is_not_re_validated() {
    // Its judge ran this build's rules already; the routing above is only
    // for logic this build cannot vouch for.
    let fixture = frozen(Some(names_live_root));
    recorded_freeze(&fixture, current());
    assert!(decide(&fixture).unwrap().reauthor.is_empty());
}

#[test]
fn an_unrecorded_freeze_is_verified_under_current_rules() {
    let fixture = frozen(None);
    let lines = decide(&fixture).unwrap().lines;
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
    let routed = decide(&frozen(Some(names_live_root))).unwrap();
    assert_eq!(routed.reauthor.len(), 1, "{routed:?}");
    assert!(
        routed
            .recorded
            .starts_with("unrecorded recorded freeze logic (no run")
    );
}

#[test]
fn nothing_is_decided_for_a_stage_that_is_not_frozen() {
    let fixture = frozen(None);
    let catalog =
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("fixture")
            .unwrap();
    let decision = launch_acceptance_decision(
        false,
        &fixture.store,
        (&fixture.project, &fixture.tasks, &fixture.project),
        &catalog,
    )
    .unwrap();
    assert_eq!(decision, LaunchAcceptance::default());
}

#[path = "workflow_decompose_frozen_chain_launch_flow_tests.rs"]
mod flow;
