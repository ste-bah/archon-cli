//! A replayed review-remediation write stands only on the tree it left.
//!
//! Its apply manifest recorded, per path, the content the patch landed (or
//! `deleted`). When the canonical tree no longer holds that content -- a
//! later stage re-delivered a file the fix deleted, an operator edit -- the
//! recorded answer is not what the repository says, and replaying it (and
//! the verdict that judged it) would count a finding resolved that the tree
//! contradicts. Drives the production write-wave seam with real Git writes.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeMap;

use archon_workflow::v2::call_data::fanout_items_for_call;
use archon_workflow::v2::script::resume_verdict::remediation_round_key;
use archon_workflow::*;
use serde_json::json;
use support::{AuditScript, Edits, Fixture, git};

fn call(ordinal: u64) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions {
        item_kind: Some("implementation".into()),
        task: Some("Post-review remediation.".into()),
        target_files_from_item: true,
        ..Default::default()
    };
    options.extra.insert(
        "remediationContract".into(),
        json!({ "version": 1, "stage": "remediate", "taskId": "TASK-001", "round": 1,
            "maxRounds": 2, "sourceReduceCallIds": ["adversarial-review-reduce"] }),
    );
    WorkflowV2HostCall {
        id: format!("review-remediate-task-001-1-{ordinal}"),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options,
    }
}

/// One wave of `call` through `store`; the number of agents dispatched.
async fn run(
    f: &Fixture,
    store: &WorkflowV2ResultStore,
    call: &WorkflowV2HostCall,
    verifiers: &[&str],
    replay: bool,
) -> usize {
    let execution = WorkflowV2CallExecution {
        call: call.clone(),
        input: json!({ "source_data": [{
            "item_id": call.id, "canonical_task_ids": ["TASK-001"],
            "task": "Post-review remediation for TASK-001. Findings (verbatim):\n[f1]",
            "target_files": ["owned.txt"], "focused_verification": [], "artifact_requirements": [],
            "artifact_verification_commands": verifiers, "work_type": "implementation",
        }] }),
        depends_on: vec![],
    };
    let edits = Edits {
        files: vec![("owned.txt", "remediated\n")],
        report: vec!["owned.txt"],
        via_adapter: false,
    };
    let items = fanout_items_for_call(&execution, &f.v2)
        .unwrap()
        .into_iter()
        .map(|item| (item, edits.clone()))
        .collect();
    let audit = Some(AuditScript {
        flagged: vec![],
        dispositions: BTreeMap::new(),
    });
    let (result, prompts) = f
        .wave_on(store, call.clone(), items, audit, &[], &[], replay)
        .await;
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            f.run.clone(),
            call.clone(),
            1,
            format!("input-{}", call.id),
            result,
            vec![],
        ))
        .unwrap();
    prompts.len()
}

fn new_session(f: &Fixture) -> WorkflowV2ResultStore {
    WorkflowV2ResultStore::new(f.v2.root().to_path_buf())
}

/// A later commit gives `owned.txt` content the fix's manifest never
/// recorded.
fn tree_moves_on(f: &Fixture) {
    std::fs::write(f.repo.join("owned.txt"), "re-delivered by a later stage\n").unwrap();
    git(&f.repo, &["commit", "-qam", "owned.txt re-delivered"]);
}

#[tokio::test]
async fn a_landed_fix_the_tree_no_longer_holds_is_not_replayed() {
    for (recorded, resumed) in [(31, 29), (31, 31)] {
        let f = Fixture::new();
        assert_eq!(run(&f, &f.v2, &call(recorded), &[], false).await, 1);
        tree_moves_on(&f);
        let session = new_session(&f);
        let dispatched = run(&f, &session, &call(resumed), &[], false).await;
        assert_eq!(
            dispatched, 1,
            "{recorded}->{resumed}: the tree contradicts the recorded fix"
        );
    }
}

#[tokio::test]
async fn a_replayed_fix_the_host_then_rejects_is_not_a_replay_its_verdict_may_follow() {
    let f = Fixture::new();
    std::fs::write(f.repo.join("marker.txt"), "present\n").unwrap();
    git(&f.repo, &["add", "marker.txt"]);
    git(&f.repo, &["commit", "-qm", "marker"]);
    let verifiers = ["test -f marker.txt"];
    assert_eq!(run(&f, &f.v2, &call(31), &verifiers, false).await, 1);
    git(&f.repo, &["rm", "-q", "marker.txt"]);
    git(&f.repo, &["commit", "-qm", "marker gone"]);
    let session = new_session(&f);
    assert_eq!(run(&f, &session, &call(29), &verifiers, true).await, 0);
    let key = remediation_round_key(&call(29)).expect("round key");
    assert_eq!(
        session.fix_replayed_from(&key),
        None,
        "the reused result failed revalidation: no verdict may follow it"
    );
}
