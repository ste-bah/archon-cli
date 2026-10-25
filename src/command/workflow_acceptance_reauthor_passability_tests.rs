//! F2 through the re-author loop: the passability judge (a replacement's
//! own pre-implementation output) returns a reply it authored but nobody
//! can use. The check it judged stays pending for a judge retry; a sibling
//! the judge never needed publishes.

use super::super::test_fixture::frozen_set_proven as frozen_set;
use super::super::*;
use super::ids;
use crate::command::workflow_task_set::executability::{
    BaselineRuns, CheckEvidence, ExecutabilityProbe,
};
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::AcceptanceContract;
use std::collections::{BTreeMap, BTreeSet};

/// A probe whose site proves every check, and whose pre-implementation
/// tree failed only `AC-F-001`: the one check the passability judge sees.
struct OneBaselineFailureProbe;

#[async_trait::async_trait]
impl ExecutabilityProbe for OneBaselineFailureProbe {
    async fn script_defects(
        &self,
        _contract: &AcceptanceContract,
        _ids: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    fn take_baseline_runs(&self) -> Option<BaselineRuns> {
        let failed = CheckResult {
            fault: None,
            acceptance_id: "AC-F-001".into(),
            environment_note: None,
            exit_code: Some(1),
            quota_walk_count: 0,
            stdout: Vec::new(),
            stderr: b"out: no such file\n".to_vec(),
            operational_error: None,
            classification: None,
        };
        Some(BaselineRuns {
            reuse: BTreeMap::new(),
            commit: "fixture-baseline".into(),
            failures: BTreeMap::from([(
                "AC-F-001".to_string(),
                CheckEvidence {
                    result: failed,
                    copy_layout: None,
                },
            )]),
            environment: BTreeMap::new(),
            forwarded: Vec::new(),
        })
    }
    fn baseline_commit(&self) -> Option<String> {
        Some("fixture-baseline".into())
    }
}

#[tokio::test]
async fn an_unusable_passability_reply_keeps_its_check_pending_while_a_sibling_publishes() {
    let set = frozen_set(&[
        ("AC-F-001", "test -f missing-a", false),
        ("AC-F-002", "test -f missing-b", false),
    ]);
    let before = set.contract();
    let named = ids(&["AC-F-001", "AC-F-002"]);
    let client =
        ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "test -s out"), |_, _| true)
            .with_evidence_reply("{ truncated");
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &named,
            gate: ReauthorGate {
                probe: &OneBaselineFailureProbe,
                seeds: &BTreeMap::new(),
            },
            trigger: "test",
        },
        &AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap(),
    )
    .await
    .expect("an unusable authored passability reply is a pending outcome, not an error");
    assert!(result.published, "the sibling publishes");
    let why = (result.unproven.get("AC-F-001"))
        .unwrap_or_else(|| panic!("the judged check stays pending: {:?}", result.unproven));
    assert!(
        why.contains("passability judge returned no usable authored verdict"),
        "{why}"
    );
    assert!(
        !result.unproven.contains_key("AC-F-002"),
        "{:?}",
        result.unproven
    );
    assert_eq!(
        set.pin().lineage.last().unwrap().reauthored_ids,
        ids(&["AC-F-002"]),
        "only the sibling is published"
    );
    assert_eq!(set.contract().acceptance[0], before.acceptance[0]);
}
