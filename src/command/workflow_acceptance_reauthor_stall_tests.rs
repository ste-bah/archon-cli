//! Issue 288: a re-authoring freeze that cannot finish ends incomplete and
//! resumable, never failed -- a check the host could not prove, even with
//! the probe's diagnostics attached, and a re-author that stopped repairing.

use super::super::test_fixture::frozen_set_proven as frozen_set;
use super::super::*;
use super::ids;
use crate::command::workflow_freeze_budget::{FREEZE_INCOMPLETE_RESUMABLE, FreezeIncomplete};
use crate::command::workflow_task_set::executability::{ExecutabilityProbe, HOST_UNPROVEN};
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

/// `freeze-acceptance --reauthor`: an unproven check with the probe's
/// diagnostics attached used to be re-wrapped as a plain error, so the CLI
/// failed hard. It is incomplete and resumable, with the diagnostics kept.
#[tokio::test]
async fn an_unproven_republish_with_diagnostics_is_incomplete_not_failed() {
    let set = frozen_set(&[("AC-F-001", "test -f missing", false)]);
    let probe = UnprovenProbe::default();
    let named = ids(&["AC-F-001"]);
    let client =
        ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "test -s out"), |_, _| true);
    let seeds = BTreeMap::new();
    let error = reauthor_and_republish(
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
        &AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd),
    )
    .await
    .expect_err("nothing unproven is published");
    let report = incomplete(error);
    assert!(report.starts_with(FREEZE_INCOMPLETE_RESUMABLE), "{report}");
    assert!(report.contains(HOST_UNPROVEN), "{report}");
    assert!(
        report.contains("AC-F-001: the scratch site could not be built"),
        "{report}"
    );
    assert!(
        report.contains("scratch policy: capture failed"),
        "the diagnostics are kept: {report}"
    );
    assert!(
        report.contains("re-running the same freeze command"),
        "{report}"
    );
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
        &AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd),
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
