use super::*;
use crate::v2::agent_adapter::{
    WorkflowV2AgentAdapter, WorkflowV2AgentError, WorkflowV2AgentRequest,
};
use crate::v2::result::WorkflowV2Result;
use crate::v2::result_store::WorkflowV2CallRecord;
use crate::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};
use serde_json::json;

const REQUEST: &str = ".archon/lab/data/datasets/spy/raw/request.json";
const BARS: &str = ".archon/lab/data/datasets/spy/raw/response.csv";

fn call(id: &str, stage: &str, task: &str, round: u64) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".to_string(),
        json!({ "version": 1, "stage": stage, "taskId": task, "round": round,
            "maxRounds": 2, "sourceReduceCallIds": ["r"] }),
    );
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options,
    }
}

fn line(stage: &str, path: &str, outcome: &str, after: &str) -> String {
    json!({ "stage_id": stage, "item_id": format!("{stage}-0"), "task_ids": ["TASK-A"],
        "path": path, "outcome": outcome, "before": "absent", "after": after, "at": 1 })
    .to_string()
}

struct World {
    _temp: tempfile::TempDir,
    project: std::path::PathBuf,
    store: WorkflowV2ResultStore,
}

/// A project whose run landed, for TASK-A's round-1 fix, an ingested
/// dataset naming a repository fixture as its source; and, for another
/// unit, a file this verdict must not be shown.
fn world() -> World {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("project")).unwrap();
    let project = temp
        .path()
        .join("project")
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let run_root = project.join(".archon/workflows/run1");
    std::fs::create_dir_all(run_root.join("write-coordination")).unwrap();
    crate::write_coordinator::project_inputs::write_test_policy(
        &run_root,
        &project,
        &[".archon/lab/data"],
    );
    let request = json!({ "fixture": "crates/lab/tests/fixtures/daily.csv", "provider": "manual",
        "observation": { "retrieved_at": "2026-01-01" } })
    .to_string();
    let hash = |text: &str| blake3::hash(text.as_bytes()).to_hex().to_string();
    for (path, text) in [(REQUEST, request.as_str()), (BARS, "d,o\n1,2\n")] {
        std::fs::create_dir_all(project.join(path).parent().unwrap()).unwrap();
        std::fs::write(project.join(path), text).unwrap();
    }
    let log = [
        line("fix-a-1", REQUEST, "intent", &hash(&request)),
        line("fix-a-1", REQUEST, "applied", &hash(&request)),
        line("fix-a-1", BARS, "applied", &hash("d,o\n1,2\n")),
        line("fix-a-1", ".archon/lab/data/refused.json", "refused", "x"),
        line("fix-b-1", ".archon/lab/data/other.json", "applied", "y"),
    ];
    std::fs::write(
        run_root.join("write-coordination/project-inputs.jsonl"),
        log.join("\n") + "\n",
    )
    .unwrap();
    let store = WorkflowV2ResultStore::new(run_root.join("v2"));
    for (id, stage, task) in [
        ("fix-a-1", "remediate", "TASK-A"),
        ("fix-b-1", "remediate", "TASK-B"),
    ] {
        let record = WorkflowV2CallRecord::new(
            "run1",
            call(id, stage, task, 1),
            1,
            "input".to_string(),
            WorkflowV2Result::accepted("fixed"),
            Vec::new(),
        );
        store.save_call_record(&record).unwrap();
    }
    World {
        _temp: temp,
        project,
        store,
    }
}

fn verify_item() -> WorkflowV2FanoutItem {
    WorkflowV2FanoutItem::read_only(
        "verification-wave-verify-a-2-0",
        "verifier",
        call("verification-wave-verify-a-2-0", "verify", "TASK-A", 2),
        json!({ "item": { "canonical_task_ids": ["TASK-A"] } }),
    )
}

#[test]
fn a_remediation_verdict_is_stamped_with_every_landing_its_unit_made() {
    let world = world();
    let items = stamp_project_data_landings(vec![verify_item()], &world.store, None);
    let stamp = stamped(&items[0].input).expect("stamped");
    let paths: Vec<&str> = stamp.landings.iter().map(|l| l.path.as_str()).collect();
    // Round 1's landings reach the round-2 verdict; the refused change and
    // the other unit's landing do not.
    assert_eq!(paths, [REQUEST, BARS]);
    let request = &stamp.landings[0];
    assert!(
        request
            .provenance
            .iter()
            .any(|field| field.contains("fixture=\"crates/lab/tests/fixtures/daily.csv\"")),
        "{request:?}"
    );
    assert_eq!(request.now, request.after);
    let section = prompt_section(&items[0].input);
    assert!(
        section.starts_with("## Project Data Landings\n"),
        "{section}"
    );
    assert!(
        section.contains(REQUEST)
            && section.contains("never a copy or an ingest of a repository test fixture"),
        "{section}"
    );
    // Batch M: a landing the project no longer holds is not the tree's, and
    // is not shown; one it still holds is.
    std::fs::remove_file(world.project.join(BARS)).unwrap();
    let items = stamp_project_data_landings(vec![verify_item()], &world.store, None);
    let stamp = stamped(&items[0].input).unwrap();
    let paths: Vec<&str> = stamp.landings.iter().map(|l| l.path.as_str()).collect();
    assert_eq!(paths, [REQUEST]);
    // Overwritten since it landed: not shown either.
    std::fs::write(world.project.join(REQUEST), "{}").unwrap();
    let items = stamp_project_data_landings(vec![verify_item()], &world.store, None);
    assert!(stamped(&items[0].input).is_none());
}

#[test]
fn a_call_that_is_not_a_remediation_verdict_is_left_alone() {
    let world = world();
    let mut item = verify_item();
    item.call = call("verification-wave-x", "remediate", "TASK-A", 1);
    let items = stamp_project_data_landings(vec![item.clone()], &world.store, None);
    assert_eq!(items[0].input, item.input);
}

fn request(input: Value) -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "verification-wave-verify-a-2-0".to_string(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        role: "verifier".to_string(),
        task: "Verify the remediation.".to_string(),
        constraints: Vec::new(),
        input,
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: Vec::new(),
        target_ownership_scopes: Vec::new(),
    }
}

/// The stamp as JSON, as the host puts it on the item.
fn stamped_input() -> Value {
    json!({
        "item": { "canonical_task_ids": ["TASK-A"] },
        PROJECT_DATA_LANDINGS_INPUT_KEY: { "landings": [
            { "path": REQUEST, "outcome": "applied", "stage_id": "fix-a-1", "item_id": "fix-a-1-0",
              "before": "absent", "after": "h1", "now": "h1",
              "provenance": ["fixture=\"crates/lab/tests/fixtures/daily.csv\""] },
            { "path": BARS, "outcome": "applied", "stage_id": "fix-a-1", "item_id": "fix-a-1-0",
              "before": "absent", "after": "h2", "now": "h2" }
        ] }
    })
}

fn verdict(status: &str, judged: Value) -> String {
    json!({
        "status": status,
        "summary": "the findings are resolved",
        "files_changed": [],
        "data": { ANSWER_KEY: judged },
        "commands_run": [{ "kind": "test", "command": "python3 check.py", "status": "succeeded",
            "exit_code": 0, "output_summary": "PASS" }],
        "task_coverage": [{ "task_id": "TASK-A", "status": "accepted", "summary": "fixed",
            "evidence": [{ "kind": "test", "summary": "check passes" }] }]
    })
    .to_string()
}

/// Fails on 25ff60622: the landings were never shown, so an accepted
/// verdict silent about them stood.
#[test]
fn an_accepted_verdict_silent_about_a_landing_is_re_asked() {
    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request(stamped_input()), &verdict("accepted", json!(null)))
        .expect_err("every landing must be judged");
    assert!(
        matches!(&error, WorkflowV2AgentError::ProjectDataLandingsUnjudged(paths) if paths == &[REQUEST.to_string(), BARS.to_string()]),
        "{error}"
    );
    assert!(
        error.to_string().contains("data.project_data_landings"),
        "{error}"
    );
    // One file judged, the other not; and a judgement with no reasons.
    let partial = json!([
        { "path": REQUEST, "legitimate": true, "provenance": "ingested by the real command" },
        { "path": BARS, "legitimate": true, "provenance": "  " }
    ]);
    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request(stamped_input()), &verdict("accepted", partial))
        .expect_err("an unreasoned judgement is none");
    assert!(
        matches!(&error, WorkflowV2AgentError::ProjectDataLandingsUnjudged(paths) if paths == &[BARS.to_string()]),
        "{error}"
    );
}

#[test]
fn an_accepted_verdict_judging_a_landing_illegitimate_is_contradicted() {
    let judged = json!([
        { "path": ".archon/lab/data/datasets/spy/", "legitimate": true, "provenance": "ingest" },
        { "path": REQUEST, "legitimate": false, "provenance": "names a repository test fixture" }
    ]);
    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request(stamped_input()), &verdict("accepted", judged))
        .expect_err("an accepted verdict cannot stand on data it judged illegitimate");
    assert!(
        matches!(&error, WorkflowV2AgentError::AcceptedWithIllegitimateProjectData(paths) if paths == &[REQUEST.to_string()]),
        "{error}"
    );
}

#[test]
fn every_landing_judged_legitimate_stands_and_a_refusal_needs_no_judgement() {
    let by_dir = json!([{ "path": ".archon/lab/data/datasets/spy/", "legitimate": true,
        "provenance": "ingested from the provider by the product's own command, per the spec" }]);
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request(stamped_input()), &verdict("accepted", by_dir))
        .expect("a directory entry covers what is under it");
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &request(stamped_input()),
            &verdict("needs_review", json!(null)),
        )
        .expect("a verdict that does not accept is free to say nothing");
    // No stamp: nothing is required.
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &request(json!({ "item": { "canonical_task_ids": ["TASK-A"] } })),
            &verdict("accepted", json!(null)),
        )
        .expect("an unstamped verdict is judged as before");
}

#[test]
fn covering_is_exact_prefix_or_all() {
    assert!(covers("a/b.json", "a/b.json"));
    assert!(covers("./a/b.json", "a/b.json"));
    assert!(covers("a/", "a/b.json"));
    assert!(covers("a", "a/b.json"));
    assert!(covers("*", "a/b.json"));
    assert!(!covers("a/b", "a/bc.json"));
    assert!(!covers("", "a/b.json"));
}

#[test]
fn a_landing_matched_to_test_material_is_judged_only_by_its_exact_path() {
    let mut input = stamped_input();
    input[PROJECT_DATA_LANDINGS_INPUT_KEY]["landings"][0]["test_material"] =
        json!(["crates/lab/tests/fixtures/daily.csv"]);
    let blanket = json!([{ "path": "*", "legitimate": true, "provenance": "all fine" }]);
    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request(input.clone()), &verdict("accepted", blanket))
        .expect_err("a blanket answer never covers flagged test material");
    assert!(
        matches!(&error, WorkflowV2AgentError::ProjectDataLandingsUnjudged(paths) if paths == &[REQUEST.to_string()]),
        "{error}"
    );
    let named = json!([
        { "path": REQUEST, "legitimate": true, "provenance": "the task spec makes this fixture the deliverable (section 3)" },
        { "path": "*", "legitimate": true, "provenance": "ingested by the product's command" }
    ]);
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request(input), &verdict("accepted", named))
        .expect("an exact, reasoned judgement of the flagged landing");
}
