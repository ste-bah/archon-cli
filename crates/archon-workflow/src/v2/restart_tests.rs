//! Issue-267 (round 2): a restart's cache invalidation and branch
//! revocation are synced to disk -- every written file and every directory
//! a name changed in -- before the state save that commits the rewind.
//! Read back from the per-thread sync journal (`durable_io::take_synced`).

use std::path::{Path, PathBuf};

use super::*;
use crate::bundle::{WorkflowBundle, WorkflowBundleOrigin};
use crate::durable_io::take_synced;
use crate::v2::{
    WorkflowV2BranchOutcome, WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2Result, WorkflowV2Status,
};

fn generated_run(temp: &tempfile::TempDir) -> (WorkflowStore, WorkflowRun) {
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = crate::spec::WorkflowSpec::from_yaml(
        "schema: archon.workflow.v1\nname: durable\ntask: t\nstages:\n  - id: wave\n    kind: agent\n",
    )
    .unwrap();
    let run = store.create_run(spec).unwrap();
    WorkflowBundle::create_for_run(
        &store,
        &run,
        "export default 1",
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
    (store, run)
}

fn call_record(attempt: u32, input: &str) -> WorkflowV2CallRecord {
    let call = WorkflowV2HostCall {
        id: "wave".into(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: Default::default(),
    };
    WorkflowV2CallRecord::new(
        "wf",
        call,
        attempt,
        input.into(),
        WorkflowV2Result::accepted("ok"),
        vec![],
    )
}

fn branch(hash: &str) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: "wave-T1".into(),
        role: "coder".into(),
        status: WorkflowV2Status::Accepted,
        result: Some(WorkflowV2Result::accepted("ok")),
        error: None,
        failure_kind: None,
        item_input_hash: Some(hash.into()),
        completion_evidence: Vec::new(),
    }
}

fn position(synced: &[PathBuf], path: &Path) -> usize {
    synced
        .iter()
        .position(|synced| synced == path)
        .unwrap_or_else(|| panic!("{} was never synced: {synced:#?}", path.display()))
}

#[test]
fn a_restart_syncs_its_invalidation_and_revocation_before_the_state_save() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp);
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    v2.save_call_record(&call_record(1, "a")).unwrap();
    v2.save_call_record(&call_record(2, "b")).unwrap();
    v2.save_branch_outcome("wave", &branch("H1")).unwrap();
    v2.save_branch_outcome("wave", &branch("H2")).unwrap();
    take_synced();

    invalidate_generated_v2_item(&store, &run, "wave", "T1").unwrap();

    let synced = take_synced();
    let state = position(
        &synced,
        &store.state_path(&run.id).with_extension("json.tmp"),
    );
    let slot = v2.result_path("wave");
    let archive = v2.call_history_dir("wave");
    let branches = v2.branch_outcome_path("wave", "wave-T1");
    let branches = branches.parent().unwrap();
    // A written file is synced as its temporary file, before the rename.
    for path in [
        slot.with_extension("json.tmp"),
        slot.parent().unwrap().to_path_buf(),
        archive.clone(),
        branches.to_path_buf(),
        branches.join("superseded"),
        branches.join("revoked"),
        v2.root().join("restart-epoch.json.tmp"),
    ] {
        assert!(
            position(&synced, &path) < state,
            "{} synced after the state save",
            path.display()
        );
    }
    // The archived record the invalidation rewrote is synced too.
    let archived = std::fs::read_dir(&archive)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(position(&synced, &archived.with_extension("json.tmp")) < state);
}
