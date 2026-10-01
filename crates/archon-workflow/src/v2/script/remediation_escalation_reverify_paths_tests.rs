//! Batch O2 (CUT-9): a re-verification plan names every moved path, and a
//! re-verification an earlier binary dispatched over the first 24 keeps
//! exactly that list, so its recorded verdict replays.

use super::*;

const MANY: usize = 30;

/// The escalated fix of a unit whose blocker is `MANY` files, all of which
/// one landing of the run changed after the refusal.
fn many_moved(world: &World) -> (WorkflowV2CallRecord, Vec<String>) {
    let judged = world.land(RUN, "review-remediate-task-a-1-1", "src/a.rs");
    world.save(
        REFUSAL,
        contract("verify", 1, false),
        false,
        refusal(Some(&judged)),
    );
    let files: Vec<String> = (0..MANY).map(|i| format!("src/m/f{i:02}.rs")).collect();
    std::fs::create_dir_all(world.repo.join("src/m")).unwrap();
    for file in &files {
        std::fs::write(world.repo.join(file), "1\n").unwrap();
    }
    world.commit(
        "archon-workflow",
        &format!("archon: wave 0 outputs (run {RUN}, stage review-remediate-task-b-1-3)"),
    );
    let baseline = world.baseline();
    world.manifest(FIX, &baseline, "idempotent_noop", &[]);
    let mut escalated = contract("remediate", 2, true);
    escalated["escalation"]["blockerPaths"] = json!(files);
    let fix = world.save(
        FIX,
        escalated,
        true,
        no_patch(WorkflowV2Status::Accepted, false),
    );
    (fix, files)
}

fn paths(plan: &Value) -> Vec<String> {
    serde_json::from_value(plan["moved_paths"].clone()).unwrap()
}

/// A recorded re-verification whose prompt quotes `listed` as moved.
fn recorded_reverify(world: &World, listed: &[String]) {
    let mut call = reverify_call(FIX, REFUSAL).call;
    call.options.task = Some(format!(
        "this run's own later landings (review-remediate-task-b-1-3) changed {}. Judge the repository as it is NOW",
        listed.join(", ")
    ));
    let record = WorkflowV2CallRecord::new(
        RUN,
        call,
        1,
        "h".into(),
        no_patch(WorkflowV2Status::Accepted, true),
        vec![],
    );
    world.store.save_call_record(&record).unwrap();
}

#[test]
fn a_reverify_plan_names_every_moved_path() {
    let world = World::new();
    let (fix, files) = many_moved(&world);
    let plan = plan_for(&world, &fix).expect("the tree moved under the refusal");
    assert_eq!(paths(&plan), files, "every moved path, none cut");
    assert_eq!(plan["landings"][0]["paths"].as_array().unwrap().len(), MANY);
    // A re-verification dispatched under the whole list keeps it.
    recorded_reverify(&world, &files);
    assert_eq!(paths(&plan_for(&world, &fix).unwrap()), files);
}

#[test]
fn a_reverify_dispatched_over_the_legacy_first_24_replays_that_list() {
    let world = World::new();
    let (fix, files) = many_moved(&world);
    let whole = plan_for(&world, &fix).unwrap();
    recorded_reverify(&world, &files[..24]);
    let resumed = plan_for(&world, &fix).unwrap();
    assert_eq!(paths(&resumed), files[..24].to_vec());
    // The call's identity -- the fix and the refusal it names -- is the same.
    assert_eq!(resumed["fix_call_id"], whole["fix_call_id"]);
    assert_eq!(resumed["refusal_call_id"], whole["refusal_call_id"]);
}
