//! D2-D4: a judge, passability judge or gate that cannot settle one
//! re-authored candidate holds that candidate pending, by whose fault it
//! is, and never blocks or discards a sibling that proved itself.

use std::collections::BTreeMap;

use super::super::test_fixture::{
    FIXTURE_JUDGE_MODEL, FrozenSet, frozen_set_in_proven, frozen_set_proven as frozen_set,
};
use super::super::*;
use super::ids;
use crate::command::workflow_task_set::reauthor::test_client::{
    SCRIPTED_PROVIDER, ScriptedAuthorJudge, command_entry,
};

/// The two checks every case repairs.
const NAMED: [&str; 2] = ["AC-F-001", "AC-F-002"];

fn request<'a>(set: &'a FrozenSet, named: &'a BTreeSet<String>) -> ReauthorRequest<'a> {
    ReauthorRequest {
        project_root: set.project.path(),
        tasks_root: &set.tasks,
        prd_path: &set.prd,
        ids: named,
        gate: set.gate(),
        trigger: "test",
    }
}

fn scope(set: &FrozenSet) -> AuthorScope {
    AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap()
}

/// AC-F-002's first reply is no entry, so the first judge call holds
/// AC-F-001 alone and every later one AC-F-002 alone.
fn staggered() -> ScriptedAuthorJudge {
    ScriptedAuthorJudge::new(
        |entry, attempt| {
            if entry["id"] == "AC-F-002" && attempt == 1 {
                "no entry here".to_string()
            } else {
                command_entry(entry, "test -s out")
            }
        },
        |_, _| true,
    )
}

fn alone(ids: &[String], id: &str) -> bool {
    ids.len() == 1 && ids[0] == id
}

/// Run the repair of [`NAMED`] in `set` with `client`; AC-F-001 must be
/// published and AC-F-002 held, its frozen entry untouched.
async fn repaired(set: &FrozenSet, client: &ScriptedAuthorJudge) -> ReauthorResult {
    let before = set.contract();
    let named = ids(&NAMED);
    let result = reauthor_and_republish(client, request(set, &named), &scope(set))
        .await
        .expect("one candidate held never blocks its proven sibling");
    assert!(result.published, "{result:?}");
    assert_eq!(
        set.pin().lineage.last().unwrap().reauthored_ids,
        ids(&["AC-F-001"])
    );
    let after = set.contract();
    assert_eq!(
        after.acceptance[0].judgment.verdict,
        archon_workflow::task_set_contract::JudgeDecision::Accepted
    );
    assert_eq!(after.acceptance[1], before.acceptance[1], "AC-F-002 held");
    assert!(result.unproven.contains_key("AC-F-002"), "{result:?}");
    result
}

#[tokio::test]
async fn a_judge_refusal_holds_its_candidate_for_its_author_and_publishes_the_sibling() {
    let set = frozen_set(&[
        ("AC-F-001", "test -f missing-a", false),
        ("AC-F-002", "test -f missing-b", false),
    ]);
    let client = staggered().with_judge_stop(|ids, passability| {
        (!passability && alone(ids, "AC-F-002")).then_some(Ok("refusal"))
    });
    let result = repaired(&set, &client).await;
    let finding = result
        .authored_pending
        .get("AC-F-002")
        .expect("authored pending");
    assert!(finding.contains("refused to judge"), "{finding}");
}

#[tokio::test]
async fn a_judge_provider_fault_holds_its_candidate_for_the_host_and_publishes_the_sibling() {
    let set = frozen_set(&[
        ("AC-F-001", "test -f missing-a", false),
        ("AC-F-002", "test -f missing-b", false),
    ]);
    let client = staggered().with_judge_stop(|ids, passability| {
        (!passability && alone(ids, "AC-F-002")).then_some(Err("upstream unavailable"))
    });
    let result = repaired(&set, &client).await;
    assert!(
        !result.authored_pending.contains_key("AC-F-002"),
        "a provider fault is never its author's: {result:?}"
    );
    assert!(
        result.unproven["AC-F-002"].contains("upstream unavailable"),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_passability_provider_fault_holds_its_candidate_for_the_host_and_publishes_the_sibling() {
    let set = frozen_set(&[
        ("AC-F-001", "test -f missing-a", false),
        ("AC-F-002", "test -f missing-b", false),
    ]);
    let client = staggered().with_judge_stop(|ids, passability| {
        (passability && alone(ids, "AC-F-002")).then_some(Err("upstream unavailable"))
    });
    let result = repaired(&set, &client).await;
    assert!(
        !result.authored_pending.contains_key("AC-F-002"),
        "a provider fault is never its author's: {result:?}"
    );
    assert!(
        result.unproven["AC-F-002"].contains("upstream unavailable"),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_passability_refusal_holds_its_candidate_for_its_author_and_publishes_the_sibling() {
    let set = frozen_set(&[
        ("AC-F-001", "test -f missing-a", false),
        ("AC-F-002", "test -f missing-b", false),
    ]);
    let client = staggered().with_judge_stop(|ids, passability| {
        (passability && alone(ids, "AC-F-002")).then_some(Ok("refusal"))
    });
    let result = repaired(&set, &client).await;
    let finding = result
        .authored_pending
        .get("AC-F-002")
        .expect("authored pending");
    assert!(finding.contains("passability judge"), "{finding}");
}

/// A floor whose verifier only tests that its own artifact exists: it runs
/// and fails before implementation, but carries a host policy defect.
fn defective_floor(set: &FrozenSet) -> archon_workflow::task_set_contract::AcceptanceCriterion {
    let mut saved = set.contract().acceptance[1].clone();
    saved.check = serde_json::from_value(serde_json::json!({
        "kind": "floor",
        "contract": {"kind": "file", "artifact_path": "out.json", "min_instances": 0, "typed_verifier_command": "test -s out.json"}
    }))
    .unwrap();
    saved.judgment.verdict = archon_workflow::task_set_contract::JudgeDecision::Accepted;
    saved
}

fn enforced_set() -> FrozenSet {
    frozen_set_in_proven(
        &[
            ("AC-F-001", "test -f missing-a", false),
            ("AC-F-002", "test -f missing-b", false),
        ],
        FreezeGateMode::Enforce,
        "",
    )
}

/// D4 end to end, under enforce: a defect one re-authored candidate carries
/// goes back to that candidate's author, never to the host and never to its
/// sibling. The candidate comes back as one an earlier attempt saved; it is
/// held to every rule a fresh reply is, so it goes back to its author before
/// the judge, the probe or the gate, and both checks publish.
#[tokio::test]
async fn under_enforce_a_defect_one_saved_candidate_carries_goes_back_to_its_author_only() {
    let set = enforced_set();
    crate::command::workflow_task_set::reauthor::cache::store(
        &set.tasks,
        &defective_floor(&set),
        &set.contract(),
        &scope(&set),
        FIXTURE_JUDGE_MODEL,
        Some(SCRIPTED_PROVIDER),
        &set.probe,
    )
    .unwrap();
    let client =
        ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "test -s out"), |_, _| true);
    let named = ids(&NAMED);
    let result = reauthor_and_republish(&client, request(&set, &named), &scope(&set))
        .await
        .expect("a defect of one candidate never blocks the republish");
    assert!(result.published && result.unproven.is_empty(), "{result:?}");
    assert_eq!(set.pin().lineage.last().unwrap().reauthored_ids, named);
    let prompts = client.prompts.lock().unwrap();
    let repair = (prompts.iter())
        .find(|prompt| prompt.contains("\"id\":\"AC-F-002\""))
        .expect("AC-F-002 went back to its author");
    assert!(repair.contains("floor is not falsifiable"), "{repair}");
    let pin = set.pin();
    assert_eq!(pin.acceptance_gate.mode, FreezeGateMode::Enforce);
    validate_full_chain(&set.tasks, &pin).unwrap();
}

/// D4 at the gate itself: before the batch publish, a candidate an enforced
/// gate would refuse for a finding that names it is held back for its
/// author, its frozen entry kept; its sibling stays to be published.
#[test]
fn hold_back_keeps_a_candidate_an_enforced_gate_refuses_out_of_the_publish() {
    let set = enforced_set();
    let named = ids(&NAMED);
    let request = request(&set, &named);
    let verified = super::super::verify::verify(&request).unwrap();
    let mut contract = verified.contract.clone();
    contract.acceptance[0].check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "test -s out".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    contract.acceptance[0].judgment.verdict =
        archon_workflow::task_set_contract::JudgeDecision::Accepted;
    contract.acceptance[1] = defective_floor(&set);
    let mut progress = crate::command::workflow_task_set::reauthor::ReauthorProgress {
        contract,
        unproven: BTreeMap::new(),
        host_faults: BTreeMap::new(),
        authored_pending: BTreeMap::new(),
        attempts: 1,
    };
    let mut publish = named.clone();
    super::super::attribute::hold_back(&request, &verified, &mut progress, &mut publish).unwrap();
    assert_eq!(publish, ids(&["AC-F-001"]));
    assert_eq!(
        progress.contract.acceptance[1],
        verified.contract.acceptance[1]
    );
    let finding = &progress.authored_pending["AC-F-002"];
    assert!(finding.contains("floor is not falsifiable"), "{finding}");
    assert_eq!(progress.unproven["AC-F-002"], *finding);
    assert!(!progress.authored_pending.contains_key("AC-F-001"));
}
