use super::*;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

fn task(id: &str, deps: &[&str], writes: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        dependency_ids: deps.iter().map(|d| d.to_string()).collect(),
        files_expected_to_change: writes.iter().map(|w| w.to_string()).collect(),
        ..Default::default()
    }
}

fn universe(tasks: Vec<WorkflowV2TaskUniverseTask>) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks,
    }
}

/// The shape that motivated this: four provider ingests, no dependency between
/// them, each writing its own directory. The authored script ran all four one
/// at a time.
#[test]
fn independent_tasks_writing_separate_files_share_one_wave() {
    let groups = author_wave_groups(&universe(vec![
        task("TASK-A-040", &[], &["data/tradingview/x.rs"]),
        task("TASK-A-050", &[], &["data/openbb/x.rs"]),
        task("TASK-A-060", &[], &["data/stooq/x.rs"]),
        task("TASK-A-070", &[], &["data/yfinance/x.rs"]),
    ]));

    assert_eq!(groups.len(), 1, "one batch expected: {groups:?}");
    assert_eq!(groups[0].task_ids.len(), 4);
}

/// A declared dependency must still serialise, or the batch would run work
/// before the thing it depends on exists.
#[test]
fn a_dependency_pushes_a_task_into_a_later_wave() {
    let groups = author_wave_groups(&universe(vec![
        task("TASK-A-010", &[], &["store.rs"]),
        task("TASK-A-040", &["TASK-A-010"], &["ingest.rs"]),
    ]));

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].task_ids, vec!["TASK-A-010"]);
    assert_eq!(groups[1].task_ids, vec!["TASK-A-040"]);
}

/// Independent by dependency but writing the same file: still separate groups,
/// because concurrent writes to one path are what the rule exists to prevent.
#[test]
fn a_shared_target_file_splits_an_otherwise_parallel_wave() {
    let groups = author_wave_groups(&universe(vec![
        task("TASK-A-001", &[], &["registry.json"]),
        task("TASK-A-002", &[], &["registry.json"]),
    ]));

    assert_eq!(groups.len(), 2, "shared write must not batch: {groups:?}");
}

/// A dependency cycle must still schedule. Refusing to plan is worse than
/// planning a cycle's members together and letting the dependency gates speak.
#[test]
fn a_dependency_cycle_still_produces_a_schedule() {
    let groups = author_wave_groups(&universe(vec![
        task("TASK-A-001", &["TASK-A-002"], &["a.rs"]),
        task("TASK-A-002", &["TASK-A-001"], &["b.rs"]),
    ]));

    let scheduled: usize = groups.iter().map(|g| g.task_ids.len()).sum();
    assert_eq!(scheduled, 2, "every task must be scheduled: {groups:?}");
}

/// An unknown dependency id cannot hold a task back forever — the universe is
/// the authority on what exists.
#[test]
fn a_dependency_outside_the_universe_is_treated_as_satisfied() {
    let groups = author_wave_groups(&universe(vec![task(
        "TASK-A-001",
        &["TASK-NOT-IN-UNIVERSE"],
        &["a.rs"],
    )]));

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].wave, 0);
}

#[test]
fn rendering_names_the_batches_for_the_brief() {
    let text = render_author_waves(&universe(vec![
        task("TASK-A-040", &[], &["x.rs"]),
        task("TASK-A-050", &[], &["y.rs"]),
    ]));

    assert!(text.contains("wave 1"), "{text}");
    assert!(text.contains("BATCH"), "{text}");
    assert!(text.contains("TASK-A-040, TASK-A-050"), "{text}");
}

#[test]
fn the_reference_shows_the_signature_of_the_primitives_it_mandates() {
    // The brief orders the mandatory reviews to use w.parallel/w.fanout and
    // w.reduce directly, forbids looking anywhere else for an example, and
    // showed neither signature — every example used the prelude helpers. A
    // live author guessed `w.reduce(spec)`, put the id inside the object, and
    // burned five of its six attempts on one identical rejection.
    let reference = super::render_dialect_reference(None);
    assert!(
        reference.contains("w.reduce('"),
        "the reference must show w.reduce called with its id first"
    );
    assert!(
        reference.contains("w.parallel('"),
        "the reference must show w.parallel called with its id first"
    );
    assert!(
        reference.contains("requires a non-empty string id"),
        "the reference must name the failure that shape prevents"
    );
}

#[test]
fn the_reference_documents_every_field_the_remediation_validator_requires() {
    // The validator rejects a review-remediation call for a missing or wrong
    // `sourceReduceCallIds`, `maxRounds`, `round`, `stage` or `taskId` — and
    // the brief named none of them. A live author invented the ids, named a
    // reduce that did not exist, and was rejected for guessing.
    let reference = super::render_dialect_reference(None);
    for field in [
        "remediationContract",
        "sourceReduceCallIds",
        "maxRounds",
        "round:",
        "taskId",
        "'remediate'",
        "'verify'",
    ] {
        assert!(
            reference.contains(field),
            "the reference must document `{field}`, which the host validates"
        );
    }
}

#[test]
fn the_brief_points_at_the_helpers_that_already_satisfy_the_review_contract() {
    // `adversarialReview` and `coverageAudit` are runtime globals that emit the
    // exact map→reduce contract the pre-flight validates. The brief ordered a
    // hand-rolled reimplementation instead, and six live drafts died on the
    // fields those helpers already fill in.
    let reference = super::render_dialect_reference(None);
    let mandate = reference
        .split("MANDATORY after all task work")
        .nth(1)
        .expect("the mandatory review section");
    let mandate = &mandate[..mandate.len().min(900)];
    assert!(mandate.contains("adversarialReview("), "{mandate}");
    assert!(mandate.contains("coverageAudit("), "{mandate}");
    assert!(mandate.contains("USE THEM"), "{mandate}");
    assert!(
        reference.contains("reduce_chunk"),
        "the chunk stage the validator checks must be named"
    );
}

#[test]
fn the_brief_treats_the_task_universe_as_the_task_set() {
    // Handed the universe as data, the brief still ordered "READ EVERY task
    // file" and a full re-derivation from disk. A live author spent 46 tool
    // calls re-reading ten files and never started writing.
    let brief = super::super::v3_author_a::V3_AUTHOR_TASK_TEMPLATE;
    assert!(
        brief.contains("AUTHORITATIVE TASK SET"),
        "the brief must name the universe as the task set"
    );
    assert!(
        !brief.contains("and EVERY task file listed"),
        "the brief must not order a full re-read of every task file"
    );
}

#[test]
fn the_rehearsal_reports_the_task_ids_a_call_claims() {
    // Empty outcomes made the dry run lie about the runtime's shape: a script
    // deriving accepted ids from its write agents' outcomes saw none, planned
    // no review map items, and was rejected for omitting every task it had
    // just implemented.
    use crate::v2::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "taskIds".to_string(),
        serde_json::json!(["TASK-A-010", "TASK-A-020"]),
    );
    let call = WorkflowV2HostCall {
        id: "agents-1".into(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options,
    };
    let payload = serde_json::json!({
        "id": "agents-1",
        "source": [{ "canonical_task_ids": ["TASK-A-010"] }, { "taskIds": ["TASK-A-020"] }],
        "options": {},
    })
    .to_string();
    let stub: serde_json::Value = serde_json::from_str(
        &super::super::dry_run_b::dry_run_stub_result(&call, &payload),
    )
    .unwrap();
    let outcomes = stub["outcomes"].as_array().expect("outcomes array");
    assert_eq!(outcomes.len(), 2, "{stub}");
    assert_eq!(outcomes[0]["status"], "accepted", "{stub}");
    assert_eq!(outcomes[0]["canonical_task_ids"][0], "TASK-A-010", "{stub}");
}
