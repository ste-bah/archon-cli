use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use archon_workflow::{
    WorkflowError, WorkflowV2HostMethod, WorkflowV2ResultStore, WorkflowV2Status,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind,
};

use super::workflow_live_task_universe::WorkflowV2TaskUniverse;

#[derive(Default)]
pub(super) struct CompletionCredit {
    pub(super) implementation: BTreeSet<String>,
    pub(super) verification: BTreeSet<String>,
    pub(super) noop: BTreeSet<String>,
}

impl CompletionCredit {
    pub(super) fn from_store(
        store: &WorkflowV2ResultStore,
    ) -> archon_workflow::WorkflowResult<Self> {
        let mut credit = Self::default();
        for record in store.load_call_records()? {
            credit.record_all(&record.completion_evidence);
        }
        for outcome in store.load_branch_outcomes()? {
            credit.record_all(&outcome.completion_evidence);
        }
        Ok(credit)
    }

    pub(super) fn completed_ids(&self) -> BTreeSet<String> {
        self.noop
            .iter()
            .chain(self.implementation.intersection(&self.verification))
            .cloned()
            .collect()
    }

    fn record_all(&mut self, evidence: &[WorkflowV2TaskCompletionEvidence]) {
        for item in evidence {
            self.record(item);
        }
    }

    pub(super) fn record(&mut self, evidence: &WorkflowV2TaskCompletionEvidence) {
        if !matches!(
            evidence.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ) {
            return;
        }
        match evidence.evidence_kind {
            WorkflowV2TaskCompletionEvidenceKind::VerifiedNoop => {
                self.noop.insert(evidence.task_id.clone());
            }
            WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate
                if evidence.status == WorkflowV2Status::Noop =>
            {
                self.noop.insert(evidence.task_id.clone());
            }
            WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate => {
                self.implementation.insert(evidence.task_id.clone());
            }
            WorkflowV2TaskCompletionEvidenceKind::FocusedVerification => {
                self.verification.insert(evidence.task_id.clone());
            }
        }
    }
}

pub(super) fn prepare_resume_credit(
    store: &WorkflowV2ResultStore,
    universe: &WorkflowV2TaskUniverse,
) -> archon_workflow::WorkflowResult<BTreeSet<String>> {
    let mut completed = CompletionCredit::from_store(store)?.completed_ids();
    apply_terminal_report_credit(store, &mut completed)?;
    let authoritative = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect::<BTreeSet<_>>();
    completed.retain(|task_id| authoritative.contains(task_id));
    archive_terminal_results(store)?;
    Ok(completed)
}

fn apply_terminal_report_credit(
    store: &WorkflowV2ResultStore,
    completed: &mut BTreeSet<String>,
) -> archon_workflow::WorkflowResult<()> {
    for record in terminal_records(store)? {
        let data = &record.result.data;
        for task_id in task_ids(data, &["accepted_tasks", "noop_tasks"]) {
            completed.insert(task_id);
        }
        for task_id in task_ids(data, &["failed_tasks", "blocked_tasks"]) {
            completed.remove(&task_id);
        }
    }
    Ok(())
}

fn terminal_records(
    store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<Vec<archon_workflow::WorkflowV2CallRecord>> {
    let mut records = store
        .load_call_records()?
        .into_iter()
        .filter(|record| record.call.method == WorkflowV2HostMethod::FinalReport)
        .collect::<Vec<_>>();
    records.sort_by(|left, right| left.finished_at.cmp(&right.finished_at));
    Ok(records)
}

fn task_ids(data: &serde_json::Value, keys: &[&str]) -> BTreeSet<String> {
    keys.iter()
        .filter_map(|key| data.get(*key).and_then(serde_json::Value::as_array))
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|task_id| !task_id.is_empty())
        .map(str::to_string)
        .collect()
}

fn archive_terminal_results(store: &WorkflowV2ResultStore) -> archon_workflow::WorkflowResult<()> {
    let records = terminal_records(store)?;
    if records.is_empty() {
        return Ok(());
    }
    let archive = resume_archive_dir(store);
    fs::create_dir_all(&archive).map_err(|source| WorkflowError::Io {
        path: archive.clone(),
        source,
    })?;
    for record in records {
        let source = store.result_path(&record.call.id);
        let target = archive.join(source.file_name().unwrap_or_default());
        fs::rename(&source, &target).map_err(|error| WorkflowError::Io {
            path: source.clone(),
            source: error,
        })?;
    }
    Ok(())
}

fn resume_archive_dir(store: &WorkflowV2ResultStore) -> PathBuf {
    let attempt = chrono::Utc::now().format("%Y%m%dT%H%M%S%fZ");
    store
        .root()
        .join("archived-resume-terminals")
        .join(attempt.to_string())
}
