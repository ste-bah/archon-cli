//! The acceptance stage's regression search, host side (Issue-114 follow-up,
//! Batch J; `archon_workflow::v2::acceptance_regression`): EVERY failed
//! command check is searched for a point of the run where it held and, from
//! there, attributed to the run landing that broke it, observed only through
//! the stage's hermetic scratch executor at exact commits. Every failed
//! check leaves here with an outcome on its record: the landing
//! (`regressed_by`), or what the search established (`regression_search`),
//! including why it could not search at all -- a stage with no scratch
//! policy runs its checks in the live checkout, which can be judged at no
//! other commit, and a declarative floor is judged against the live project
//! only.

use std::collections::BTreeMap;
use std::path::PathBuf;

use archon_workflow::acceptance_world::FrozenCommandRef;
use archon_workflow::task_set_contract::AcceptanceCriterion;
use archon_workflow::v2::WorkflowV2ResultStore;
use archon_workflow::v2::acceptance_regression::{
    CheckObserver, FailingCheck, RegressionSearchV1, SearchBudget, attribute_regressions,
    command_fingerprint, failure_signature,
};
use archon_workflow::v2::acceptance_routing::check_command;
use archon_workflow::v2::acceptance_stage::AcceptanceRoundRecordV1;

use super::exec::{StageContext, command_reference, git_head, observe_in_scratch_at};
use crate::command::acceptance_scratch_policy::NativeBinding;

pub(super) struct ScratchObserver<'a> {
    pub(super) context: &'a StageContext,
    pub(super) binding: &'a NativeBinding,
    pub(super) refs: Vec<FrozenCommandRef>,
    pub(super) evidence_dir: PathBuf,
}

#[async_trait::async_trait]
impl CheckObserver for ScratchObserver<'_> {
    async fn observe(&self, commit: &str, ids: &[String]) -> Option<BTreeMap<String, bool>> {
        let refs: Vec<FrozenCommandRef> = self
            .refs
            .iter()
            .filter(|reference| ids.contains(&reference.acceptance_id))
            .cloned()
            .collect();
        if refs.is_empty() {
            return None;
        }
        let at: String = commit.chars().take(12).collect();
        let checks = observe_in_scratch_at(
            self.context,
            self.binding,
            commit,
            &refs,
            &self.evidence_dir.join(format!("regression-{at}")),
        )
        .await
        .ok()?;
        // A check that could not be evaluated there has no verdict.
        Some(
            checks
                .into_iter()
                .filter(|check| check.operational_error.is_none())
                .map(|check| (check.acceptance_id, check.exit_code == Some(0)))
                .collect(),
        )
    }
}

/// Record, on each failed check of `record`, the run landing that broke it
/// or what the search established instead, within `budget` (the stage
/// passes [`SearchBudget::default`]).
pub(super) async fn attribute(
    context: &StageContext,
    criteria: &[&AcceptanceCriterion],
    chain_digest: &str,
    run_dir: &std::path::Path,
    evidence_dir: &std::path::Path,
    record: &mut AcceptanceRoundRecordV1,
    budget: SearchBudget,
) {
    let commands: BTreeMap<&str, (&AcceptanceCriterion, String)> = criteria
        .iter()
        .map(|criterion| {
            (
                criterion.id.as_str(),
                (*criterion, check_command(criterion)),
            )
        })
        .collect();
    let mut failing: Vec<FailingCheck> = Vec::new();
    for check in record
        .checks
        .iter_mut()
        .filter(|check| check.ran_and_failed())
    {
        let note = match commands.get(check.check_id.as_str()) {
            None => Some("its frozen command is not among the checks the round ran"),
            Some((criterion, _)) if command_reference(criterion, chain_digest).is_none() => Some(
                "it is a declarative floor, judged against the live project only, so no earlier point of the run can be probed",
            ),
            Some(_) => None,
        };
        if let Some(note) = note {
            check.regression_search = Some(RegressionSearchV1::not_searched(note));
            continue;
        }
        let command = commands[check.check_id.as_str()].1.as_str();
        failing.push(FailingCheck {
            id: check.check_id.clone(),
            owners: check.owning_tasks.clone(),
            signature: failure_signature(check.exit_code, &check.stderr_tail, &check.stdout_tail),
            fingerprint: command_fingerprint(command),
        });
    }
    let unsearched = |record: &mut AcceptanceRoundRecordV1, note: &str| {
        for check in &mut record.checks {
            if failing.iter().any(|failed| failed.id == check.check_id) {
                check.regression_search = Some(RegressionSearchV1::not_searched(note));
            }
        }
    };
    if failing.is_empty() {
        return;
    }
    let Some(binding) = &context.binding else {
        return unsearched(
            record,
            "the stage runs its checks in the live checkout (no [workflow.acceptance_execution] scratch policy), so no earlier point of the run can be probed",
        );
    };
    let (Some(tip), _) = git_head(&context.repository) else {
        return unsearched(
            record,
            "the target repository's HEAD could not be read, so the run's landings could not be searched",
        );
    };
    let refs: Vec<FrozenCommandRef> = criteria
        .iter()
        .filter(|criterion| failing.iter().any(|check| check.id == criterion.id))
        .filter_map(|criterion| command_reference(criterion, chain_digest))
        .collect();
    let observer = ScratchObserver {
        context,
        binding,
        refs,
        evidence_dir: evidence_dir.to_path_buf(),
    };
    let store = WorkflowV2ResultStore::new(run_dir.join("v2"));
    let found = attribute_regressions(
        &store,
        &context.repository,
        &tip,
        &failing,
        &observer,
        budget,
    )
    .await;
    for check in &mut record.checks {
        if let Some(regression) = found.regressions.get(&check.check_id) {
            check.regressed_by = Some(regression.clone());
        } else if let Some(search) = found.searches.get(&check.check_id) {
            check.regression_search = Some(search.clone());
        }
    }
}
