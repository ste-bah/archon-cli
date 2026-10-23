//! Issue 288: a re-authoring freeze that cannot finish ends incomplete and
//! resumable, never failed -- a check the host could not prove, even with
//! the probe's diagnostics logged separately, and a re-author that stopped repairing.

use super::super::test_fixture::frozen_set_proven as frozen_set;
use super::super::*;
use super::ids;
use crate::command::workflow_freeze_budget::{FREEZE_INCOMPLETE_RESUMABLE, FreezeIncomplete};
use crate::command::workflow_task_set::executability::ExecutabilityProbe;
use crate::command::workflow_task_set::reauthor::REAUTHOR_ATTEMPTS;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use crate::command::workflow_task_set::unproven_incomplete;
use archon_workflow::task_set_contract::AcceptanceContract;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

/// A probe that runs nothing: every accepted check is unproven, with a
/// diagnostic saying why, as a host whose scratch site broke reports it.
#[derive(Default)]
struct UnprovenProbe {
    unproven: Mutex<BTreeMap<String, String>>,
}

#[derive(Default)]
struct OneUnprovenProbe {
    unproven: Mutex<BTreeMap<String, String>>,
}

#[async_trait::async_trait]
impl ExecutabilityProbe for OneUnprovenProbe {
    async fn script_defects(
        &self,
        _contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        if let Some(id) = ["AC-F-001", "AC-X-001"]
            .into_iter()
            .find(|id| ids.contains(*id))
        {
            self.unproven.lock().unwrap().insert(
                id.into(),
                "the mutation copy had an operational I/O failure".into(),
            );
        }
        BTreeMap::new()
    }
    fn take_unproven(&self) -> BTreeMap<String, String> {
        std::mem::take(&mut *self.unproven.lock().unwrap())
    }
    fn baseline_commit(&self) -> Option<String> {
        Some("fixture-baseline".into())
    }
}

#[tokio::test]
async fn one_host_unproven_check_does_not_discard_two_proven_repairs() {
    let set = frozen_set(&[
        ("AC-F-001", "test -f missing-a", false),
        ("AC-F-002", "test -f missing-b", false),
        ("AC-F-003", "test -f missing-c", false),
    ]);
    let before = set.contract_bytes();
    let named = ids(&["AC-F-001", "AC-F-002", "AC-F-003"]);
    let probe = OneUnprovenProbe::default();
    let client =
        ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "test -s out"), |_, _| true);
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &named,
            gate: ReauthorGate {
                probe: &probe,
                seeds: &BTreeMap::new(),
            },
            trigger: "test",
        },
        &AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap(),
    )
    .await
    .expect("the host-unproven entry remains pending while its siblings publish");
    assert!(result.published);
    assert_eq!(
        result.unproven["AC-F-001"],
        "the mutation copy had an operational I/O failure"
    );
    assert_eq!(
        set.contract().acceptance[0],
        serde_json::from_slice::<AcceptanceContract>(&before)
            .unwrap()
            .acceptance[0]
    );
    assert_ne!(set.contract_bytes(), before);
    let pin = set.pin();
    let transition = pin.lineage.last().expect("published repair lineage");
    assert_eq!(
        transition.reauthored_ids,
        ids(&["AC-F-002", "AC-F-003"]),
        "unproven entries are not recorded as published"
    );
}

#[tokio::test]
async fn enforce_partial_republish_publishes_proven_siblings_and_keeps_pending_ids() {
    let set = super::super::test_fixture::frozen_set_in(
        &[
            ("AC-X-001", "test -f missing-a", false),
            ("AC-X-002", "test -f missing-b", false),
            ("AC-X-003", "test -f missing-c", false),
        ],
        archon_workflow::task_set_contract::FreezeGateMode::Enforce,
        "",
    );
    let before = set.contract_bytes();
    let named = ids(&["AC-X-001", "AC-X-002", "AC-X-003"]);
    let probe = OneUnprovenProbe::default();
    let client =
        ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "test -s out"), |_, _| true);
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &named,
            gate: ReauthorGate {
                probe: &probe,
                seeds: &BTreeMap::new(),
            },
            trigger: "test",
        },
        &AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap(),
    )
    .await
    .expect("pending check must not block proven siblings under enforcement");
    assert!(result.published);
    assert!(result.unproven.contains_key("AC-X-001"), "{result:?}");
    assert_eq!(set.contract().acceptance[0].id, "AC-X-001");
    assert_ne!(set.contract_bytes(), before);
    assert_eq!(
        set.pin().lineage.last().unwrap().reauthored_ids,
        ids(&["AC-X-002", "AC-X-003"])
    );
    assert!(
        !non_accepted_ids(&set.contract()).is_empty(),
        "pending ids remain visible"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn enforce_timeout_stays_pending_while_proven_siblings_publish() {
    let mut set = super::super::test_fixture::frozen_set_in_proven(
        &[
            ("AC-X-001", "check-with-no-progress", false),
            ("AC-X-002", "check-two", false),
            ("AC-X-003", "check-three", false),
        ],
        archon_workflow::task_set_contract::FreezeGateMode::Enforce,
        "",
    );
    set.probe = set.probe.with_check_cap(1);
    let named = ids(&["AC-X-001", "AC-X-002", "AC-X-003"]);
    let client = ScriptedAuthorJudge::new(
        |entry, _| {
            command_entry(
                entry,
                if entry["id"] == "AC-X-001" {
                    "sleep 5; test -s out"
                } else {
                    "test -s out"
                },
            )
        },
        |_, _| true,
    );
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &named,
            gate: ReauthorGate {
                probe: &set.probe,
                seeds: &BTreeMap::new(),
            },
            trigger: "test",
        },
        &AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap(),
    )
    .await
    .expect("host-unproven timeout must not block siblings");
    assert!(result.published);
    assert!(result.unproven.contains_key("AC-X-001"), "{result:?}");
    assert_eq!(
        set.pin().lineage.last().unwrap().reauthored_ids,
        ids(&["AC-X-002", "AC-X-003"])
    );
    assert!(!non_accepted_ids(&set.contract()).is_empty());
}

#[tokio::test]
async fn an_unaccepted_authored_check_stays_pending_while_accepted_siblings_publish() {
    let set = frozen_set(&[
        ("AC-F-001", "test -f missing-a", false),
        ("AC-F-002", "test -f missing-b", false),
        ("AC-F-003", "test -f missing-c", false),
    ]);
    let before = set.contract();
    let named = ids(&["AC-F-001", "AC-F-002", "AC-F-003"]);
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -s out"),
        |id, _| id != "AC-F-001",
    );
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &named,
            gate: set.gate(),
            trigger: "test",
        },
        &AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap(),
    )
    .await
    .expect("an authored refusal remains per-check pending");
    assert!(result.published);
    assert!(
        result.unproven.contains_key("AC-F-001"),
        "{:?}",
        result.unproven
    );
    assert_eq!(client.authored(), 3 + REAUTHOR_ATTEMPTS);
    assert_eq!(
        set.pin().lineage.last().unwrap().reauthored_ids,
        ids(&["AC-F-002", "AC-F-003"])
    );
    assert_eq!(set.contract().acceptance[0], before.acceptance[0]);
}

#[tokio::test]
async fn malformed_judge_replies_keep_each_authored_check_pending_for_retry() {
    let set = frozen_set(&[
        ("AC-F-001", "test -f missing-a", false),
        ("AC-F-002", "test -f missing-b", false),
    ]);
    let named = ids(&["AC-F-001", "AC-F-002"]);
    let client =
        ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "test -s out"), |_, _| true)
            .with_judge_reply("{ truncated");
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &named,
            gate: set.gate(),
            trigger: "test",
        },
        &AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap(),
    )
    .await
    .expect("malformed judge text is a pending authored outcome");
    assert!(!result.published);
    assert_eq!(
        result.unproven.keys().cloned().collect::<BTreeSet<_>>(),
        named
    );
    assert!(
        result
            .unproven
            .values()
            .all(|finding| finding.contains("judge returned no usable authored verdict"))
    );
    assert_eq!(
        client.authored(),
        2,
        "the unusable judge reply retries each same candidate without spending another author attempt"
    );
}

#[async_trait::async_trait]
impl ExecutabilityProbe for UnprovenProbe {
    async fn script_defects(
        &self,
        _contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        let mut unproven = self.unproven.lock().unwrap();
        for id in ids {
            unproven.insert(id.clone(), "the scratch site could not be built".into());
        }
        BTreeMap::new()
    }
    fn take_diagnostics(&self) -> Vec<String> {
        vec!["scratch policy: capture failed".into()]
    }
    fn take_unproven(&self) -> BTreeMap<String, String> {
        std::mem::take(&mut *self.unproven.lock().unwrap())
    }
}

fn incomplete(error: anyhow::Error) -> String {
    let error = unproven_incomplete(error);
    let incomplete = FreezeIncomplete::caused(&error)
        .unwrap_or_else(|| panic!("a hard error, not an incomplete freeze: {error:#}"));
    incomplete.report()
}

/// `freeze-acceptance --reauthor`: an unproven check keeps its typed stop
/// reason while probe diagnostics are emitted as separate log lines.
#[tokio::test]
async fn an_all_unproven_republish_returns_a_pending_partial_result_without_publishing() {
    let set = frozen_set(&[("AC-F-001", "test -f missing", false)]);
    let probe = UnprovenProbe::default();
    let named = ids(&["AC-F-001"]);
    let client =
        ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "test -s out"), |_, _| true);
    let seeds = BTreeMap::new();
    let before = set.contract_bytes();
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &named,
            gate: ReauthorGate {
                probe: &probe,
                seeds: &seeds,
            },
            trigger: "test",
        },
        &AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap(),
    )
    .await
    .expect("unproven checks are reported as pending, not a run error");
    assert!(!result.published);
    assert_eq!(
        result.unproven["AC-F-001"],
        "the scratch site could not be built"
    );
    assert_eq!(
        set.contract_bytes(),
        before,
        "nothing is changed when all are pending"
    );
    assert_eq!(result.diagnostics, vec!["scratch policy: capture failed"]);
    assert_eq!(client.authored(), 1, "the author is not asked again");
}

/// A re-author whose every attempt is refuted stops after its no-progress
/// window. That is a stall: the freeze is incomplete and resumable, naming
/// the pending check and its last finding, never a hard error.
#[tokio::test]
async fn a_re_author_that_stops_repairing_is_incomplete_not_failed() {
    let set = frozen_set(&[("AC-F-001", "test -f missing", false)]);
    let named = ids(&["AC-F-001"]);
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "true"), |_, _| false);
    let contract = set.contract();
    let seeds = BTreeMap::new();
    let error = crate::command::workflow_task_set::reauthor::reauthor(
        &client,
        &contract,
        &named,
        &AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap(),
        "sonnet",
        &ReauthorGate {
            probe: &UnprovenProbe::default(),
            seeds: &seeds,
        },
    )
    .await
    .expect_err("a refuted check is never published");
    let stalled = error
        .downcast_ref::<crate::command::workflow_task_set::reauthor::ReauthorStalled>()
        .unwrap_or_else(|| panic!("an untyped stall: {error:#}"));
    assert_eq!(stalled.pending, ["AC-F-001"]);
    let report = incomplete(error);
    assert!(report.starts_with(FREEZE_INCOMPLETE_RESUMABLE), "{report}");
    assert!(report.contains("repaired no named check"), "{report}");
    assert!(report.contains("still pending: AC-F-001"), "{report}");
    assert_eq!(
        client.authored(),
        crate::command::workflow_task_set::reauthor::REAUTHOR_ATTEMPTS,
        "one attempt per idle window slot"
    );
}

/// Any other error stays what it was: a freeze is incomplete only on a stall.
#[test]
fn an_unrelated_error_is_returned_unchanged() {
    let error = unproven_incomplete(anyhow::anyhow!("the PRD is unreadable"));
    assert!(FreezeIncomplete::caused(&error).is_none());
    assert_eq!(format!("{error:#}"), "the PRD is unreadable");
}
