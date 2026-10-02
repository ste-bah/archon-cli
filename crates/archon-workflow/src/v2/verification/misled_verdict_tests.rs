use super::*;
use crate::v2::result::WorkflowV2Result;
use crate::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};
use serde_json::json;

const INPUT: &str = ".archon/lab/data";
const GONE: &str = ".archon/lab/data/sets/sample/rows.csv";
const HELD: &str = ".archon/lab/data/spec.json";
const FIX: &str = "review-remediate-task-a-1-1";
const VERIFY: &str = "verification-wave-review-verify-task-a-1-2";

fn hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap()
}

fn rfc(ns: i64) -> String {
    chrono::DateTime::from_timestamp_nanos(ns).to_rfc3339()
}

fn call(id: &str, stage: &str) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".to_string(),
        json!({ "version": 1, "stage": stage, "taskId": "TASK-A", "round": 1,
            "maxRounds": 1, "sourceReduceCallIds": ["r"], "observedBy": ["run-1"] }),
    );
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options,
    }
}

struct World {
    _temp: tempfile::TempDir,
    project: PathBuf,
    store: WorkflowV2ResultStore,
    records: Vec<WorkflowV2CallRecord>,
}

fn line(path: &str, outcome: &str, after: &str, at: i64) -> String {
    json!({ "stage_id": FIX, "item_id": format!("{FIX}-0"), "task_ids": ["TASK-A"],
        "path": path, "outcome": outcome, "before": "absent", "after": after, "at": at })
    .to_string()
}

/// One fix id run three times: its first execution landed `GONE`, which
/// was taken out by hand before its last execution was seeded; the last
/// landed `HELD`, which the project holds. `seeded_gone` is the state the
/// last seed recorded for `GONE`.
fn world(seeded_gone: Option<&str>, answers: Value) -> World {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("project")).unwrap();
    let project = temp
        .path()
        .join("project")
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let run_root = project.join(".archon/workflows/run1");
    crate::write_coordinator::project_inputs::write_test_policy(&run_root, &project, &[INPUT]);
    let start = now_ns() - 600 * 1_000_000_000;
    std::fs::create_dir_all(project.join(HELD).parent().unwrap()).unwrap();
    std::fs::write(project.join(HELD), "held").unwrap();
    let log = [
        line(GONE, "applied", &hash("rows"), start),
        line(HELD, "synced", &hash("held"), now_ns() + 1_000),
    ];
    std::fs::create_dir_all(run_root.join("write-coordination")).unwrap();
    std::fs::write(
        run_root.join("write-coordination/project-inputs.jsonl"),
        log.join("\n") + "\n",
    )
    .unwrap();
    let mut files = BTreeMap::new();
    if let Some(state) = seeded_gone {
        files.insert(GONE.to_string(), state.to_string());
    }
    let seed = SeedRecord {
        project: project.clone(),
        inputs: vec![INPUT.to_string()],
        files,
        ..SeedRecord::default()
    };
    write_json(&seed_path(&run_root, FIX, &format!("{FIX}-0")), &seed).unwrap();
    let store = WorkflowV2ResultStore::new(run_root.join("v2"));
    let fix = WorkflowV2CallRecord::new(
        "run1",
        call(FIX, "remediate"),
        3,
        "i".into(),
        WorkflowV2Result::accepted("fixed"),
        vec![],
    );
    let mut refused = WorkflowV2Result::accepted("refused");
    refused.status = WorkflowV2Status::NeedsReview;
    refused.data = json!({ "items": [{ "data": { ANSWER_KEY: answers } }] });
    let mut verdict = WorkflowV2CallRecord::new(
        "run1",
        call(VERIFY, "verify"),
        1,
        "i".into(),
        refused,
        vec![],
    );
    verdict.status = WorkflowV2Status::NeedsReview;
    verdict.finished_at = rfc(now_ns() + 60 * 1_000_000_000);
    World {
        _temp: temp,
        project,
        store,
        records: vec![fix, verdict],
    }
}

fn answer(path: &str, legitimate: bool) -> Value {
    json!({ "path": path, "legitimate": legitimate, "provenance": "p" })
}

fn misled(world: &World) -> bool {
    verdict_misled(&world.store, &world.records[1], &world.records)
}

use std::collections::{BTreeMap, BTreeSet};

#[test]
fn a_refusal_of_data_its_fixs_own_seed_saw_gone_is_misled() {
    let world = world(None, json!([answer(GONE, false), answer(HELD, true)]));
    assert!(!world.project.join(GONE).exists());
    assert!(misled(&world));
}

#[test]
fn a_refusal_that_also_judges_held_data_is_a_verdict() {
    let world = world(None, json!([answer(GONE, false), answer(HELD, false)]));
    assert!(!misled(&world));
    // A blanket answer covers the held landing too.
    let world = world_with(json!([answer("*", false)]));
    assert!(!misled(&world));
    let world = world_with(json!([answer(".archon/lab/data/", false)]));
    assert!(!misled(&world));
}

fn world_with(answers: Value) -> World {
    world(None, answers)
}

#[test]
fn data_the_seed_still_saw_in_place_was_there_when_the_verdict_ran() {
    // Taken out after the verdict, not before: the verdict judged it.
    let world = world(Some(&hash("rows")), json!([answer(GONE, false)]));
    assert!(!misled(&world));
}

#[test]
fn a_seed_taken_after_the_verdict_proves_nothing() {
    let mut world = world(None, json!([answer(GONE, false)]));
    world.records[1].finished_at = rfc(now_ns() - 300 * 1_000_000_000);
    assert!(!misled(&world));
}

#[test]
fn an_acceptance_or_a_refusal_naming_no_data_is_never_misled() {
    let mut world = world(None, json!([answer(GONE, false)]));
    world.records[1].status = WorkflowV2Status::Accepted;
    assert!(!misled(&world));
    let world = world_with(json!([answer(HELD, true)]));
    assert!(!misled(&world));
    let world = world_with(json!([answer(".archon/lab/other.json", false)]));
    assert!(
        !misled(&world),
        "an answer that covers nothing it was shown"
    );
}

#[test]
fn what_a_verdict_was_shown_is_read_from_its_kept_view_first() {
    // The seed would prove nothing; the kept view says the file was gone.
    let world = world(Some(&hash("rows")), json!([answer(GONE, false)]));
    let gone = Landing {
        path: GONE.into(),
        after: hash("rows"),
        now: "absent".into(),
        ..Landing::default()
    };
    let stamp = LandingsStamp {
        landings: vec![gone.clone()],
        ..LandingsStamp::default()
    };
    record_shown(&world.store, VERIFY, &format!("{VERIFY}-0"), Some(&stamp));
    assert!(misled(&world));
    // Shown in place: the verdict judged it.
    let held = LandingsStamp {
        landings: vec![Landing {
            now: gone.after.clone(),
            ..gone
        }],
        ..LandingsStamp::default()
    };
    record_shown(&world.store, VERIFY, &format!("{VERIFY}-0"), Some(&held));
    assert!(!misled(&world));
    // Shown nothing: nothing it refused was a landing.
    record_shown(&world.store, VERIFY, &format!("{VERIFY}-0"), None);
    assert!(!misled(&world));
}

#[test]
fn a_severe_signal_about_anything_else_keeps_the_refusal() {
    let mut world = world(None, json!([answer(GONE, false), answer(HELD, true)]));
    let gap = |description: &str| crate::v2::result::WorkflowV2ResidualGap {
        id: "g".into(),
        description: description.into(),
        severity: Some("blocking".into()),
    };
    world.records[1].result.residual_gaps = vec![gap(&format!("{GONE} is a hand-made sample"))];
    assert!(misled(&world), "a signal about the refused data");
    world.records[1]
        .result
        .residual_gaps
        .push(gap("the parser drops rows"));
    assert!(!misled(&world), "a refusal of its own");
}

#[test]
fn refused_paths_share_their_directory_below_the_top_level() {
    let refused: BTreeSet<String> = [
        ".archon/lab/data/sets/sample/rows.csv",
        ".archon/lab/data/sets/sample/raw/request.json",
    ]
    .map(String::from)
    .into();
    assert!(needles(&refused).contains(&".archon/lab/data/sets/sample".to_string()));
    let apart: BTreeSet<String> = ["a/x", "b/y"].map(String::from).into();
    assert_eq!(needles(&apart).len(), 2);
}
