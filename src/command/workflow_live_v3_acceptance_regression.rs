//! The acceptance stage's regression search, host side (Issue-114 follow-up,
//! `archon_workflow::v2::acceptance_regression`): each failing command check
//! that held at its owner's landing is attributed to the run landing that
//! broke it, observed only through the stage's hermetic scratch executor at
//! exact commits. A stage with no scratch policy runs its checks in the live
//! checkout, which can be judged at no other commit: nothing is attributed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use archon_workflow::acceptance_world::FrozenCommandRef;
use archon_workflow::task_set_contract::AcceptanceCriterion;
use archon_workflow::v2::WorkflowV2ResultStore;
use archon_workflow::v2::acceptance_regression::{
    CheckObserver, FailingCheck, attribute_regressions,
};
use archon_workflow::v2::acceptance_stage::{AcceptanceCheckStatus, AcceptanceRoundRecordV1};

use super::exec::{StageContext, command_reference, git_head, observe_in_scratch_at};
use crate::command::acceptance_scratch_policy::NativeBinding;

struct ScratchObserver<'a> {
    context: &'a StageContext,
    binding: &'a NativeBinding,
    refs: Vec<FrozenCommandRef>,
    evidence_dir: PathBuf,
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

/// Name, on each failing command check of `record` that regressed, the run
/// landing that broke it.
pub(super) async fn attribute(
    context: &StageContext,
    criteria: &[&AcceptanceCriterion],
    chain_digest: &str,
    run_dir: &std::path::Path,
    evidence_dir: &std::path::Path,
    record: &mut AcceptanceRoundRecordV1,
) {
    let Some(binding) = &context.binding else {
        return;
    };
    let (Some(tip), _) = git_head(&context.repository) else {
        return;
    };
    let failing: Vec<FailingCheck> = record
        .checks
        .iter()
        .filter(|check| check.status == AcceptanceCheckStatus::Failed && check.kind == "command")
        .map(|check| FailingCheck {
            id: check.check_id.clone(),
            owners: check.owning_tasks.clone(),
        })
        .collect();
    if failing.is_empty() {
        return;
    }
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
    let found = attribute_regressions(&store, &context.repository, &tip, &failing, &observer).await;
    for check in &mut record.checks {
        if let Some(regression) = found.get(&check.check_id) {
            check.regressed_by = Some(regression.clone());
        }
    }
}
